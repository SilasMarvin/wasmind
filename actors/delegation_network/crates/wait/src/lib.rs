use delegation_network_common_types::AgentSpawned;
use std::collections::HashMap;
use wasmind_actor_utils::{
    common_messages::{
        actors::Exit,
        assistant::{
            RequestStatusUpdate, Section, Status, SystemPromptContent, SystemPromptContribution,
            WaitReason,
        },
        tools::{
            ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, ToolsAvailable,
            UIDisplayInfo,
        },
    },
    llm_client_types::{Tool, ToolFunctionDefinition},
    messages::Message,
};

#[allow(warnings)]
mod bindings;

#[derive(Clone, Debug)]
enum AgentStatus {
    Active,
    Shutdown,
}

wasmind_actor_utils::actors::macros::generate_actor_trait!();

const WAIT_USAGE_GUIDE: &str = r#"## wait Tool - Resume Waiting After an Interruption

**Purpose**: To PAUSE your execution and RESUME waiting for agent responses after you have been interrupted by an external event (like a new user message).

**CRITICAL: `wait` (Tool) vs. `spawn_agent(wait=true)` (Parameter)**
- Use `spawn_agent(..., wait=true)` to **INITIATE** a waiting period when you first delegate a task.
- Use the `wait()` tool to **RESUME** a waiting period **AFTER** you have been woken up and have finished handling the interruption.

**DO NOT use `wait()` right after `spawn_agent(..., wait=true)`. It is incorrect.**

**Correct Workflow Example**:
1. You need a long task completed. You call `spawn_agent(task="Run the full integration test suite", wait=true)`.
2. While you are waiting, the **user** sends a new message: "Hey, while that's running, can you quickly check the version of Python installed?"
3. You delegate the user's request: `spawn_agent(task="Run 'python --version' and report the output", type="Worker", wait=true)`. After it finishes, you report back to the user.
4. Now that you've handled the user's interruption, you must go back to waiting for the original test suite to finish. You call `wait(reason="Resuming wait for the integration test suite to complete.")`.

**Best Practice**: Always include a clear reason for why you're waiting so the user understands the current state."#;

#[derive(wasmind_actor_utils::actors::macros::Actor)]
struct WaitValidator {
    scope: String,
    tracked_agents: HashMap<String, AgentStatus>,
}

#[derive(Debug, serde::Deserialize)]
struct WaitInput {
    reason: String,
}

impl GeneratedActorTrait for WaitValidator {
    fn new(scope: String, _config: String) -> Self {
        // Broadcast guidance about how to use the wait tool
        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "wait:usage_guide".to_string(),
            content: SystemPromptContent::Text(WAIT_USAGE_GUIDE.to_string()),
            priority: 700,
            section: Some(Section::Tools),
        });

        // Broadcast the wait tool to make it available
        let wait_tool = Tool {
            tool_type: "function".to_string(),
            function: ToolFunctionDefinition {
                name: "wait".to_string(),
                description: "Pause and wait for a new system or subordinate agent message. Use this to coordinate with other agents or wait for external input.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "reason": {
                            "type": "string",
                            "description": "Reason for waiting"
                        }
                    }
                }),
            },
        };

        let _ = Self::broadcast_common_message(ToolsAvailable {
            tools: vec![wait_tool],
        });

        Self {
            scope,
            tracked_agents: HashMap::new(),
        }
    }

    fn handle_message(
        &mut self,
        message: bindings::exports::wasmind::actor::actor::MessageEnvelope,
    ) {
        // Handle AgentSpawned messages to track new agents
        // Only track agents spawned from our scope
        if message.from_scope == self.scope
            && let Some(agent_spawned) = Self::parse_as::<AgentSpawned>(&message)
        {
            self.tracked_agents
                .insert(agent_spawned.agent_id.clone(), AgentStatus::Active);
            return;
        }

        // Handle Exit messages to mark agents as shutdown
        // Only handle exits from agents we're tracking
        if let Some(_exit) = Self::parse_as::<Exit>(&message) {
            // Mark the agent that sent the exit message as shutdown
            if let Some(status) = self.tracked_agents.get_mut(&message.from_scope) {
                *status = AgentStatus::Shutdown;
            }
            return;
        }

        // Handle wait tool calls from our own scope
        if message.from_scope == self.scope {
            if let Some(execute_tool) = Self::parse_as::<ExecuteTool>(&message) {
                if execute_tool.tool_call.function.name == "wait" {
                    self.handle_wait_tool_call(execute_tool);
                }
            }
        }
    }

    fn destructor(&mut self) {}
}

impl WaitValidator {
    fn handle_wait_tool_call(&mut self, tool_call: ExecuteTool) {
        // Parse the tool parameters (optional)
        let params: WaitInput = match serde_json::from_str(&tool_call.tool_call.function.arguments)
        {
            Ok(params) => params,
            Err(e) => {
                let error_result = ToolCallResult {
                    content: format!("Error parsing parameters: {e}"),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "Error parsing parameters".to_string(),
                        expanded: Some(format!("Error parsing parameters: {e:?}")),
                    },
                };
                self.send_error_result(
                    &tool_call.tool_call.id,
                    &tool_call.originating_request_id,
                    error_result,
                );
                return;
            }
        };

        // Check if we have any active subordinate agents to wait for
        let active_agent_count = self
            .tracked_agents
            .values()
            .filter(|status| matches!(status, AgentStatus::Active))
            .count();

        if active_agent_count == 0 {
            let error_result = ToolCallResult {
                content: "You have no active subordinate agents to wait for. Use spawn_agent to create agents first, or complete your current task.".to_string(),
                ui_display_info: UIDisplayInfo {
                    collapsed: "No agents to wait for".to_string(),
                    expanded: Some("Error: No active subordinate agents\n\nYou cannot wait when you have no active subordinate agents. Either:\n1. Use spawn_agent to create agents first\n2. Complete your current task instead of waiting".to_string()),
                },
            };
            self.send_error_result(
                &tool_call.tool_call.id,
                &tool_call.originating_request_id,
                error_result,
            );
            return;
        }

        // Create a status update request to put the agent into wait mode
        let status_update_request = RequestStatusUpdate {
            agent: self.scope.clone(),
            status: Status::Wait {
                reason: WaitReason::WaitingForAgentCoordination {
                    originating_request_id: tool_call.originating_request_id.clone(),
                    coordinating_tool_name: "wait".to_string(),
                    target_agent_scope: None,
                    user_can_interrupt: true,
                },
            },
            originating_request_id: Some(tool_call.originating_request_id.clone()),
        };

        // Send the status update request
        let _ = Self::broadcast_common_message(status_update_request);

        // Send success result
        let result = ToolCallResult {
            content: "Waiting...".to_string(),
            ui_display_info: UIDisplayInfo {
                collapsed: format!("Waiting: {}", params.reason),
                expanded: Some(format!(
                    "Waiting for system input\n\nReason: {}",
                    params.reason
                )),
            },
        };

        self.send_success_result(
            &tool_call.tool_call.id,
            &tool_call.originating_request_id,
            result,
        );
    }

    fn send_error_result(
        &self,
        tool_call_id: &str,
        originating_request_id: &str,
        error_result: ToolCallResult,
    ) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: originating_request_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(error_result),
            },
        };

        bindings::wasmind::actor::messaging::broadcast(
            ToolCallStatusUpdate::MESSAGE_TYPE,
            &serde_json::to_string(&update).unwrap().into_bytes(),
        );
    }

    fn send_success_result(
        &self,
        tool_call_id: &str,
        originating_request_id: &str,
        result: ToolCallResult,
    ) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: originating_request_id.to_string(),
            status: ToolCallStatus::Done { result: Ok(result) },
        };

        bindings::wasmind::actor::messaging::broadcast(
            ToolCallStatusUpdate::MESSAGE_TYPE,
            &serde_json::to_string(&update).unwrap().into_bytes(),
        );
    }
}

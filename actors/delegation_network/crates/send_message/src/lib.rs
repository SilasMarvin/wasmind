use delegation_network_common_types::AgentSpawned;
use std::collections::HashMap;
use wasmind_actor_utils::{
    common_messages::{
        actors::Exit,
        assistant::{
            AddMessage, RequestStatusUpdate, Section, Status, SystemPromptContent,
            SystemPromptContribution, WaitReason,
        },
        tools::{
            ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, ToolsAvailable,
            UIDisplayInfo,
        },
    },
    llm_client_types::ChatMessage,
    llm_client_types::{Tool, ToolFunctionDefinition},
    messages::Message,
};

#[allow(warnings)]
mod bindings;

const SEND_MESSAGE_USAGE_GUIDE: &str = r#"## send_message Tool - Communicate with Subordinate Agents

**Purpose**: Send messages to agents you've spawned to provide guidance, ask questions, or give updates.

**When to Use**:
- ✅ Agent seems stuck or hasn't provided updates recently
- ✅ Need to provide course correction or additional context  
- ✅ Want to check status on long-running operations
- ✅ Need to give new instructions based on changed requirements

**When NOT to Use**:
- ❌ Right after spawning an agent (let them work first!)
- ❌ Asking for final results (you get these automatically)
- ❌ Micromanaging - agents work best when given autonomy

**Message Guidelines**:
- Be clear and specific about what you need
- Provide context for why you're reaching out
- Include any new information that might help
- Set clear expectations for response

**Examples**:
- "I noticed you haven't provided updates in 30 minutes. What's your current status on the web scraping task?"
- "The client just requested we also include product images in the CSV. Can you add an 'image_url' column?"
- "Great progress so far! For the next phase, please focus on error handling for network timeouts."

**wait Parameter**: Set to `true` if you need to pause and wait for their response before continuing."#;

#[derive(Clone, Debug)]
enum AgentStatus {
    Active,
    Shutdown,
}

#[derive(serde::Deserialize)]
struct SendMessageInput {
    agent_id: String,
    message: String,
    wait: Option<bool>,
}

wasmind_actor_utils::actors::macros::generate_actor_trait!();

#[derive(wasmind_actor_utils::actors::macros::Actor)]
struct SendMessageValidator {
    scope: String,
    tracked_agents: HashMap<String, AgentStatus>,
}

impl GeneratedActorTrait for SendMessageValidator {
    fn new(scope: String, _config: String) -> Self {
        // Broadcast guidance about how to use the send_message tool
        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "send_message:usage_guide".to_string(),
            content: SystemPromptContent::Text(SEND_MESSAGE_USAGE_GUIDE.to_string()),
            priority: 900,
            section: Some(Section::Tools),
        });

        // Broadcast the send_message tool to make it available
        let send_message_tool = Tool {
            tool_type: "function".to_string(),
            function: ToolFunctionDefinition {
                name: "send_message".to_string(),
                description: "Send a message to a subordinate agent. Use this to communicate with agents that you have spawned or that are working under your management.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "agent_id": {
                            "type": "string",
                            "description": "The ID of the agent to send the message to"
                        },
                        "message": {
                            "type": "string",
                            "description": "The message to send"
                        },
                        "wait": {
                            "type": "boolean",
                            "description": "If `true` pause and wait for a response else continue performing actions (default `false`)"
                        }
                    },
                    "required": ["agent_id", "message"]
                }),
            },
        };

        let _ = Self::broadcast_common_message(ToolsAvailable {
            tools: vec![send_message_tool],
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

        // Handle send_message tool calls from our own scope
        if message.from_scope == self.scope {
            if let Some(execute_tool) = Self::parse_as::<ExecuteTool>(&message) {
                if execute_tool.tool_call.function.name == "send_message" {
                    self.handle_send_message_tool_call(execute_tool);
                }
            }
        }
    }

    fn destructor(&mut self) {}
}

impl SendMessageValidator {
    fn handle_send_message_tool_call(&mut self, tool_call: ExecuteTool) {
        let params: SendMessageInput =
            match serde_json::from_str(&tool_call.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    let error_msg = format!("Failed to parse send message parameters: {}", e);
                    let error_result = ToolCallResult {
                        content: error_msg.clone(),
                        ui_display_info: UIDisplayInfo {
                            collapsed: "Parameters: Invalid format".to_string(),
                            expanded: Some(format!(
                                "Error: Failed to parse parameters\n\nDetails: {}",
                                error_msg
                            )),
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

        // Check if agent exists and its status
        match self.tracked_agents.get(&params.agent_id) {
            None => {
                let error_result = ToolCallResult {
                    content: format!(
                        "Agent '{}' does not exist. You can only send messages to agents you have spawned.",
                        params.agent_id
                    ),
                    ui_display_info: UIDisplayInfo {
                        collapsed: format!("Agent '{}' does not exist", params.agent_id),
                        expanded: Some(format!(
                            "Error: Agent does not exist\n\nAgent ID: {}\n\nYou can only send messages to agents that you have spawned using the spawn_agent tool.",
                            params.agent_id
                        )),
                    },
                };
                self.send_error_result(
                    &tool_call.tool_call.id,
                    &tool_call.originating_request_id,
                    error_result,
                );
                return;
            }
            Some(AgentStatus::Shutdown) => {
                let error_result = ToolCallResult {
                    content: format!(
                        "Agent '{}' is shutdown and you can no longer message it. The agent has completed its task or been terminated.",
                        params.agent_id
                    ),
                    ui_display_info: UIDisplayInfo {
                        collapsed: format!("Agent '{}' is shutdown", params.agent_id),
                        expanded: Some(format!(
                            "Error: Agent is shutdown\n\nAgent ID: {}\n\nThis agent has completed its task or been terminated. You cannot send messages to shutdown agents.",
                            params.agent_id
                        )),
                    },
                };
                self.send_error_result(
                    &tool_call.tool_call.id,
                    &tool_call.originating_request_id,
                    error_result,
                );
                return;
            }
            Some(AgentStatus::Active) => {
                // Agent exists and is active, proceed with sending the message
                self.send_message_to_agent(params, tool_call);
            }
        }
    }

    fn send_message_to_agent(&self, params: SendMessageInput, tool_call: ExecuteTool) {
        let add_message = AddMessage {
            agent: params.agent_id.clone(),
            message: ChatMessage::system(&params.message),
        };

        // Deliver message to the target agent
        let _ = Self::broadcast_common_message(add_message);

        // If wait is true, send a status update request to make the agent wait for a response
        if params.wait.unwrap_or(false) {
            let status_update_request = RequestStatusUpdate {
                agent: self.scope.clone(),
                status: Status::Wait {
                    reason: WaitReason::WaitingForAgentCoordination {
                        originating_request_id: tool_call.originating_request_id.clone(),
                        coordinating_tool_name: "send_message".to_string(),
                        target_agent_scope: Some(params.agent_id.clone()),
                        user_can_interrupt: true,
                    },
                },
                originating_request_id: Some(tool_call.originating_request_id.clone()),
            };

            let _ = Self::broadcast_common_message(status_update_request);
        }

        // Create success result
        let success_message = format!(
            "Message sent to agent {} - please allow at least 5 minutes for a response.",
            params.agent_id
        );

        let result = ToolCallResult {
            content: success_message.clone(),
            ui_display_info: UIDisplayInfo {
                collapsed: format!(
                    "To {}: Message delivered{}",
                    params.agent_id,
                    if params.wait.unwrap_or(false) {
                        " (waiting)"
                    } else {
                        ""
                    }
                ),
                expanded: Some(format!(
                    "Recipient: {}\nWaiting for response: {}\n\nMessage: {}",
                    params.agent_id,
                    params.wait.unwrap_or(false),
                    params.message
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


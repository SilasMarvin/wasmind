use wasmind_actor_utils::{
    common_messages::{
        assistant::{RequestStatusUpdate, Section, Status, SystemPromptContent, SystemPromptContribution, WaitReason},
        tools::{ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, UIDisplayInfo},
    },
    messages::Message,
    tools,
};

#[allow(warnings)]
mod bindings;

const WAIT_USAGE_GUIDE: &str = r#"## wait Tool - Intelligent Timing Coordination

**Purpose**: Pause execution to wait for agent responses or coordinate timing between operations.

**IMPORTANT**: You wake automatically on important events - trust the system!

**When to Use**:
- ✅ ONLY when waiting for agents to finish their tasks
- ✅ After sending a message with `wait: true` and need response
- ✅ Coordinating sequential operations that depend on each other
- ✅ When you need to pause before proceeding to next phase

**When NOT to Use**:
- ❌ Arbitrary delays or "just in case" waiting
- ❌ Immediately after spawning agents (let them work!)
- ❌ When no specific coordination is needed

**How It Works**:
- Sets your status to "waiting" 
- System will wake you when relevant events occur
- User can interrupt the wait if needed
- You'll receive updates automatically when agents complete tasks

**Examples**:
- "Waiting for the Python developer to finish the web scraping implementation"
- "Pausing until the database setup is complete before starting the API development"
- "Waiting for confirmation from the DevOps team about deployment readiness"

**Best Practice**: Always include a clear reason for why you're waiting so the user understands the current state."#;

#[derive(tools::macros::Tool)]
#[tool(
    name = "wait",
    description = "Pause and wait for a new system or subordinate agent message. Use this to coordinate with other agents or wait for external input.",
    schema = r#"{
        "type": "object",
        "properties": {
            "reason": {
                "type": "string",
                "description": "Optional reason for waiting (for logging/display purposes)"
            }
        }
    }"#
)]
struct WaitTool {
    scope: String,
}

#[derive(Debug, serde::Deserialize)]
struct WaitInput {
    reason: Option<String>,
}

impl tools::Tool for WaitTool {
    fn new(scope: String, _config: String) -> Self {
        // Broadcast guidance about how to use the wait tool
        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "wait:usage_guide".to_string(),
            content: SystemPromptContent::Text(WAIT_USAGE_GUIDE.to_string()),
            priority: 700,
            section: Some(Section::Tools),
        });

        Self { scope }
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        // Parse the tool parameters (optional)
        let params: WaitInput = match serde_json::from_str(&tool_call.tool_call.function.arguments) {
            Ok(params) => params,
            Err(_) => WaitInput { reason: None },
        };

        let wait_reason = params.reason.unwrap_or_else(|| "Waiting for system input".to_string());

        // Create a status update request to put the agent into wait mode
        let status_update_request = RequestStatusUpdate {
            agent: self.scope.clone(),
            status: Status::Wait {
                reason: WaitReason::WaitingForAgentCoordination {
                    originating_request_id: tool_call.originating_request_id.clone(),
                    coordinating_tool_call_id: tool_call.tool_call.id.clone(),
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
                collapsed: format!("Waiting: {}", wait_reason),
                expanded: Some(format!("Waiting for system input\n\nReason: {}", wait_reason)),
            },
        };

        let update = ToolCallStatusUpdate {
            id: tool_call.tool_call.id,
            originating_request_id: tool_call.originating_request_id,
            status: ToolCallStatus::Done {
                result: Ok(result),
            },
        };

        bindings::wasmind::actor::messaging::broadcast(
            ToolCallStatusUpdate::MESSAGE_TYPE,
            &serde_json::to_string(&update).unwrap().into_bytes(),
        );
    }
}
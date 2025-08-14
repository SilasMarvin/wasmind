use bindings::wasmind::actor::agent::get_parent_scope;
use wasmind_actor_utils::{
    common_messages::{
        assistant::{
            AddMessage, RequestStatusUpdate, Section, Status, SystemPromptContent,
            SystemPromptContribution, WaitReason,
        },
        tools::{ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, UIDisplayInfo},
    },
    llm_client_types::ChatMessage,
    messages::Message,
    tools,
};

#[allow(warnings)]
mod bindings;

const SEND_MANAGER_MESSAGE_USAGE_GUIDE: &str = r#"## send_manager_message Tool - Escalate to Your Manager

**Purpose**: Send messages upward to your direct manager when you need guidance, are blocked, or have critical updates.

**When to Use**:
- ✅ You're blocked and need guidance to proceed
- ✅ Critical errors or unexpected situations have occurred
- ✅ You need clarification on requirements or priorities
- ✅ You've discovered important information that affects the overall plan
- ✅ You need additional resources or permissions

**When NOT to Use**:
- ❌ Regular status updates (managers get these automatically)
- ❌ Minor questions you can resolve independently
- ❌ Frequent check-ins without specific need
- ❌ Asking for approval on routine decisions

**Message Guidelines**:
- Be specific about what you need from your manager
- Provide context for the situation
- Include what you've already tried (if blocked)
- Suggest possible solutions if you have them
- Be clear about urgency level

**Examples**:
- "I'm blocked on the API integration - the authentication endpoint is returning 401 errors despite using the provided credentials. I've tried regenerating the token. Can you help verify the correct authentication method?"
- "I've discovered that the database migration will require 6+ hours of downtime. This conflicts with the 2pm deployment deadline. Should I proceed or adjust the timeline?"
- "The client requirements document mentions features X and Y, but our current sprint only accounts for X. Should I implement Y as well, or clarify scope first?"

**wait Parameter**: Set to `true` if you need to pause all work and wait for manager's response before continuing."#;

#[derive(Debug, serde::Deserialize)]
struct SendManagerMessageInput {
    message: String,
    wait: Option<bool>,
}

#[derive(tools::macros::Tool)]
#[tool(
    name = "send_manager_message",
    description = "Send a message to your direct manager. Use this to escalate issues, ask for guidance when blocked, or communicate critical updates that require management attention.",
    schema = r#"{
        "type": "object",
        "properties": {
            "message": {
                "type": "string",
                "description": "The message to send to your manager"
            },
            "wait": {
                "type": "boolean",
                "description": "If `true` pause all work and wait for a response from your manager before continuing (default `false`)"
            }
        },
        "required": ["message"]
    }"#
)]
struct SendManagerMessageTool {
    scope: String,
}

impl tools::Tool for SendManagerMessageTool {
    fn new(scope: String, _config: String) -> Self {
        // Broadcast guidance about how to use the send_manager_message tool
        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "send_manager_message:usage_guide".to_string(),
            content: SystemPromptContent::Text(SEND_MANAGER_MESSAGE_USAGE_GUIDE.to_string()),
            priority: 900,
            section: Some(Section::Tools),
        });

        Self { scope }
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        // Parse the tool parameters
        let params: SendManagerMessageInput =
            match serde_json::from_str(&tool_call.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    let error_msg =
                        format!("Failed to parse send manager message parameters: {}", e);
                    let error_result = ToolCallResult {
                        content: error_msg.clone(),
                        ui_display_info: UIDisplayInfo {
                            collapsed: "Parameter Error".to_string(),
                            expanded: Some(format!("Parameter Error:\n{}", error_msg)),
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

        // Get parent scope to send message to manager
        let parent_scope = match get_parent_scope() {
            Some(parent) => parent,
            None => {
                let error_msg =
                    "No manager available - you are at the top level of the agent hierarchy";
                let error_result = ToolCallResult {
                    content: error_msg.to_string(),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "No Manager Available".to_string(),
                        expanded: Some(error_msg.to_string()),
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

        // Create the message to send to the manager
        let add_message = AddMessage {
            agent: parent_scope.clone(),
            message: ChatMessage::system(&params.message),
        };

        // Send the message to the manager
        let _ = Self::broadcast_common_message(add_message);

        // If wait is true, send a status update request to make the agent wait for a response
        if params.wait.unwrap_or(false) {
            let status_update_request = RequestStatusUpdate {
                agent: self.scope.clone(),
                status: Status::Wait {
                    reason: WaitReason::WaitingForAgentCoordination {
                        originating_request_id: tool_call.originating_request_id.clone(),
                        coordinating_tool_name: "send_manager_message".to_string(),
                        target_agent_scope: Some(parent_scope.clone()),
                        user_can_interrupt: true,
                    },
                },
                originating_request_id: Some(tool_call.originating_request_id.clone()),
            };

            let _ = Self::broadcast_common_message(status_update_request);
        }

        // Create success result
        let success_message =
            format!("Message sent to your manager - please allow time for a response.");

        let result = ToolCallResult {
            content: success_message.clone(),
            ui_display_info: UIDisplayInfo {
                collapsed: "Message sent to manager".to_string(),
                expanded: Some(format!("Message sent to manager:\n\n{}", params.message)),
            },
        };

        self.send_success_result(
            &tool_call.tool_call.id,
            &tool_call.originating_request_id,
            result,
        );
    }
}

impl SendManagerMessageTool {
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


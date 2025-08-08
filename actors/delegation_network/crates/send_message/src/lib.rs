use hive_actor_utils::{
    common_messages::{
        assistant::{AddMessage, RequestStatusUpdate, Section, Status, SystemPromptContent, SystemPromptContribution, WaitReason},
        tools::{ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, UIDisplayInfo},
    },
    llm_client_types::ChatMessage,
    messages::Message,
    tools,
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

#[derive(Debug, serde::Deserialize)]
struct SendMessageInput {
    agent_id: String,
    message: String,
    wait: Option<bool>,
}

#[derive(tools::macros::Tool)]
#[tool(
    name = "send_message",
    description = "Send a message to a subordinate agent. Use this to communicate with agents that you have spawned or that are working under your management.",
    schema = r#"{
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
    }"#
)]
struct SendMessageTool {
    scope: String,
}

impl tools::Tool for SendMessageTool {
    fn new(scope: String, _config: String) -> Self {
        // Broadcast guidance about how to use the send_message tool
        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "send_message:usage_guide".to_string(),
            content: SystemPromptContent::Text(SEND_MESSAGE_USAGE_GUIDE.to_string()),
            priority: 900,
            section: Some(Section::Tools),
        });

        Self { scope }
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        let params: SendMessageInput = match serde_json::from_str(&tool_call.tool_call.function.arguments) {
            Ok(params) => params,
            Err(e) => {
                let error_msg = format!("Failed to parse send message parameters: {}", e);
                let error_result = ToolCallResult {
                    content: error_msg.clone(),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "Parameters: Invalid format".to_string(),
                        expanded: Some(format!("Error: Failed to parse parameters\n\nDetails: {}", error_msg)),
                    },
                };
                self.send_error_result(&tool_call.tool_call.id, error_result);
                return;
            }
        };

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
                        coordinating_tool_call_id: tool_call.tool_call.id.clone(),
                        coordinating_tool_name: "send_message".to_string(),
                        target_agent_scope: Some(params.agent_id.clone()),
                        user_can_interrupt: true,
                    },
                },
                tool_call_id: Some(tool_call.tool_call.id.clone()),
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
                collapsed: format!("To {}: Message delivered{}", 
                    params.agent_id,
                    if params.wait.unwrap_or(false) { " (waiting)" } else { "" }
                ),
                expanded: Some(format!("Recipient: {}\nWaiting for response: {}\n\nMessage: {}", 
                    params.agent_id, 
                    params.wait.unwrap_or(false),
                    params.message
                )),
            },
        };

        self.send_success_result(&tool_call.tool_call.id, result);
    }
}

impl SendMessageTool {
    fn send_error_result(&self, tool_call_id: &str, error_result: ToolCallResult) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(error_result),
            },
        };

        bindings::hive::actor::messaging::broadcast(
            ToolCallStatusUpdate::MESSAGE_TYPE,
            &serde_json::to_string(&update).unwrap().into_bytes(),
        );
    }

    fn send_success_result(&self, tool_call_id: &str, result: ToolCallResult) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done {
                result: Ok(result),
            },
        };

        bindings::hive::actor::messaging::broadcast(
            ToolCallStatusUpdate::MESSAGE_TYPE,
            &serde_json::to_string(&update).unwrap().into_bytes(),
        );
    }
}

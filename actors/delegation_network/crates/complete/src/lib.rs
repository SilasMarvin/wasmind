use bindings::hive::actor::agent::get_parent_scope;
use hive_actor_utils::{
    common_messages::{
        assistant::{AddMessage, AgentTaskResponse, RequestStatusUpdate, Status},
        tools::{ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, UIDisplayInfo},
    },
    llm_client_types::SystemChatMessage,
    messages::Message,
    tools,
};

#[allow(warnings)]
mod bindings;

#[derive(Debug, serde::Deserialize)]
struct CompleteInput {
    summary: String,
    success: bool,
}

#[derive(tools::macros::Tool)]
#[tool(
    name = "complete",
    description = "Call this tool when you have completed your assigned task. Use this to provide a summary of what was accomplished and signal that the task is finished.",
    schema = r#"{
        "type": "object",
        "properties": {
            "summary": {
                "type": "string",
                "description": "A brief summary of what was accomplished"
            },
            "success": {
                "type": "boolean",
                "description": "Whether the task was completed successfully (true) or failed (false)"
            }
        },
        "required": ["summary", "success"]
    }"#
)]
struct CompleteTool {
    scope: String,
}

impl tools::Tool for CompleteTool {
    fn new(scope: String, _config: String) -> Self {
        Self { scope }
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        // Parse the tool parameters
        let params: CompleteInput =
            match serde_json::from_str(&tool_call.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    let error_msg = format!("Failed to parse complete parameters: {}", e);
                    let error_result = ToolCallResult {
                        content: error_msg.clone(),
                        ui_display_info: UIDisplayInfo {
                            collapsed: "❌ Parameter Error".to_string(),
                            expanded: Some(format!("❌ Parameter Error:\n{}", error_msg)),
                        },
                    };
                    self.send_error_result(&tool_call.tool_call.id, error_result);
                    return;
                }
            };

        // Send status update request to mark the agent as done
        let status_update_request = RequestStatusUpdate {
            agent: self.scope.clone(),
            status: Status::Done {
                result: Ok(AgentTaskResponse {
                    summary: params.summary.clone(),
                    success: params.success,
                }),
            },
            tool_call_id: Some(tool_call.tool_call.id.clone()),
        };

        let _ = Self::broadcast_common_message(status_update_request);

        // Create success result
        let result_message = format!(
            "Task completed{}",
            if params.success {
                " successfully"
            } else {
                " with failures"
            }
        );

        if let Some(parent_scope) = get_parent_scope() {
            let system_message_content = format!(
                "Agent {} has completed its task {}. Summary: {}",
                self.scope,
                if params.success {
                    "successfully"
                } else {
                    "with failures"
                },
                params.summary
            );

            let _ = Self::broadcast_common_message(AddMessage {
                agent: parent_scope,
                message: hive_actor_utils::llm_client_types::ChatMessage::System(
                    SystemChatMessage {
                        content: system_message_content,
                    },
                ),
            });
        }

        let status_text = if params.success {
            "Completed"
        } else {
            "Failed"
        };

        let result = ToolCallResult {
            content: result_message,
            ui_display_info: UIDisplayInfo {
                collapsed: format!("{}: {}", status_text, params.summary),
                expanded: Some(params.summary.clone()),
            },
        };

        self.send_success_result(&tool_call.tool_call.id, result);
    }
}

impl CompleteTool {
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
            status: ToolCallStatus::Done { result: Ok(result) },
        };

        bindings::hive::actor::messaging::broadcast(
            ToolCallStatusUpdate::MESSAGE_TYPE,
            &serde_json::to_string(&update).unwrap().into_bytes(),
        );
    }
}

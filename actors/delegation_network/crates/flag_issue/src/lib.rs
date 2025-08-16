use wasmind_actor_utils::{
    common_messages::{
        assistant::{
            AddMessage, AgentTaskResponse, QueueStatusChange,
            RequestStatusUpdate, Status, WaitReason,
        },
        tools::{ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, UIDisplayInfo},
    },
    llm_client_types::{ChatMessage, SystemChatMessage},
    tools,
};

#[allow(warnings)]
mod bindings;

#[derive(Debug, serde::Deserialize)]
struct FlagIssueParams {
    issue_summary: String,
}

#[derive(tools::macros::Tool)]
#[tool(
    name = "flag_issue",
    description = "Flag that the analyzed agent appears to be stuck, looping, or having issues",
    schema = r#"{
        "type": "object",
        "properties": {
            "issue_summary": {
                "type": "string", 
                "description": "A brief summary of why the agent seems problematic. Example: 'Agent is repeatedly trying the same failed action' or 'Agent is making no progress toward the goal'"
            }
        },
        "required": ["issue_summary"]
    }"#
)]
struct FlagIssueTool {
    scope: String,
}

impl tools::Tool for FlagIssueTool {
    fn new(scope: String, _config: String) -> Self {
        Self { scope }
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        // Parse the tool parameters
        let params: FlagIssueParams =
            match serde_json::from_str(&tool_call.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    let error_msg = format!("Failed to parse flag_issue parameters: {}", e);
                    self.send_error_result(&tool_call.tool_call.id, &tool_call.originating_request_id, error_msg);
                    return;
                }
            };

        // Get parent scope (the agent being monitored) and grandparent scope (the manager)
        let parent_scope = bindings::wasmind::actor::agent::get_parent_scope();
        let grandparent_scope = parent_scope
            .as_ref()
            .and_then(|p| bindings::wasmind::actor::agent::get_parent_scope_of(p));

        // Interrupt the monitored agent (parent of health checker)
        if let Some(parent_scope) = parent_scope
            && let Some(grandparent_scope) = grandparent_scope
        {
            let _ = Self::broadcast_common_message(QueueStatusChange {
                agent: parent_scope.clone(),
                status: Status::Wait {
                    reason: WaitReason::WaitingForSystemInput {
                        required_scope: Some(grandparent_scope.clone()),
                        interruptible_by_user: true,
                    },
                },
            });

            let _ = Self::broadcast_common_message(AddMessage {
                agent: grandparent_scope,
                message: ChatMessage::System(SystemChatMessage {
                    content: format!(
                        "HEALTH CHECK ALERT: Your spawned agent '{}' has been flagged for problematic behavior and has been temporarily paused.\n\nIssue: {}\n\nThe agent is waiting for your guidance before continuing. Please review its recent actions and provide direction.",
                        parent_scope, params.issue_summary
                    ),
                }),
            });
        }

        // Send success result
        self.send_success_result(&tool_call.tool_call.id, &tool_call.originating_request_id, &params.issue_summary);
    }
}

impl FlagIssueTool {
    fn send_error_result(&self, tool_call_id: &str, originating_request_id: &str, error_msg: String) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: originating_request_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(ToolCallResult {
                    content: error_msg.clone(),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "Parameters: Invalid format".to_string(),
                        expanded: Some(format!("Error: Failed to parse parameters\n\nDetails: {}", error_msg)),
                    },
                }),
            },
        };

        let _ = Self::broadcast_common_message(update);
    }

    fn send_success_result(&self, tool_call_id: &str, originating_request_id: &str, issue_summary: &str) {
        let status_update_request = RequestStatusUpdate {
            agent: self.scope.clone(),
            status: Status::Done {
                result: Ok(AgentTaskResponse {
                    summary: format!("Flagged parent agent for issue: {issue_summary}"),
                    success: true,
                }),
            },
            originating_request_id: Some(originating_request_id.to_string()),
        };

        let _ = Self::broadcast_common_message(status_update_request);

        let result = ToolCallResult {
            content: format!("Agent flagged for issue: {}", issue_summary),
            ui_display_info: UIDisplayInfo {
                collapsed: format!("Issue reported: {}", 
                    if issue_summary.len() > 40 { 
                        format!("{}...", &issue_summary[..37])
                    } else { 
                        issue_summary.to_string()
                    }
                ),
                expanded: Some(format!("Operation: Report problem\nAgent paused for manager review\n\nIssue: {}", issue_summary)),
            },
        };

        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: originating_request_id.to_string(),
            status: ToolCallStatus::Done { result: Ok(result) },
        };

        let _ = Self::broadcast_common_message(update);
    }
}

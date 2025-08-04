use hive_actor_utils::{
    common_messages::{
        actors::Exit,
        assistant::{AddMessage, InterruptAndForceWaitForSystemInput},
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
                    self.send_error_result(&tool_call.tool_call.id, error_msg);
                    return;
                }
            };

        // Get parent scope (the agent being monitored) and grandparent scope (the manager)
        let parent_scope = bindings::hive::actor::agent::get_parent_scope();
        let grandparent_scope = parent_scope
            .as_ref()
            .and_then(|p| bindings::hive::actor::agent::get_parent_scope_of(p));

        // Interrupt the monitored agent (parent of health checker)
        if let Some(parent_scope) = parent_scope
            && let Some(grandparent_scope) = grandparent_scope
        {
            let _ = Self::broadcast_common_message(InterruptAndForceWaitForSystemInput {
                agent: parent_scope.clone(),
                required_scope: Some(grandparent_scope.clone()),
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
        let result = ToolCallResult {
            content: format!("Agent flagged for issue: {}", params.issue_summary),
            ui_display_info: UIDisplayInfo {
                collapsed: "Issue flagged".to_string(),
                expanded: Some(format!(
                    "Agent flagged for problematic behavior: {}",
                    params.issue_summary
                )),
            },
        };

        self.send_success_result(&tool_call.tool_call.id, result);
    }
}

impl FlagIssueTool {
    fn send_error_result(&self, tool_call_id: &str, error_msg: String) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(ToolCallResult {
                    content: error_msg.clone(),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "Error".to_string(),
                        expanded: Some(error_msg),
                    },
                }),
            },
        };

        let _ = Self::broadcast_common_message(update);
    }

    fn send_success_result(&self, tool_call_id: &str, result: ToolCallResult) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done { result: Ok(result) },
        };

        let _ = Self::broadcast_common_message(update);

        let _ = Self::broadcast_common_message(Exit);
    }
}


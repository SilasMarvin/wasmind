use crate::actors::{ActorContext, ActorMessage, AgentMessage, Message, ToolCallResult};
use crate::llm_client::ToolCall;
use crate::scope::Scope;
use tokio::sync::broadcast;

use crate::actors::tools::Tool;

pub fn format_flag_issue_for_review_manager_message(flagged_agent: &Scope, issue: &str) -> String {
    format!(
        r#"Agent: {flagged_agent}\n has been flagged by the system health check. It has been automatically stopped and is waiting for a message from you.\nReason: {issue}"#
    )
}

/// Tool for temporal agents to flag issues for review
#[derive(hive_macros::ActorContext)]
pub struct FlagIssueForReviewTool {
    tx: broadcast::Sender<ActorMessage>,
    scope: Scope,
    og_scope: Scope,
    og_parent_scope: Scope,
}

impl FlagIssueForReviewTool {
    pub fn new(
        tx: broadcast::Sender<ActorMessage>,
        scope: Scope,
        og_scope: Scope,
        og_parent_scope: Scope,
    ) -> Self {
        Self {
            tx,
            scope,
            og_scope,
            og_parent_scope,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct FlagIssueParams {
    issue_summary: String,
}

#[async_trait::async_trait]
impl Tool for FlagIssueForReviewTool {
    const TOOL_NAME: &str = "flag_issue_for_review";
    const TOOL_DESCRIPTION: &str =
        "Flags that the analyzed agent appears to be stuck or in a loop.";
    const TOOL_INPUT_SCHEMA: &str = r#"{
        "type": "object",
        "properties": {
            "issue_summary": {
                "type": "string",
                "description": "A one-sentence summary of why the agent seems stuck. Example: 'The agent is repeatedly trying to access a file that does not exist.'"
            }
        },
        "required": ["issue_summary"]
    }"#;

    type Params = FlagIssueParams;

    async fn execute_tool_call(&mut self, tool_call: ToolCall, params: Self::Params) {
        self.broadcast_with_scope(
            &self.og_scope,
            Message::Agent(AgentMessage {
                agent_id: self.og_scope.clone(),
                message: crate::actors::AgentMessageType::InterAgentMessage(
                    crate::actors::InterAgentMessage::InterruptAndForceWaitForManager {
                        tool_call_id: tool_call.id.clone(),
                    },
                ),
            }),
        );

        self.broadcast_with_scope(
            &self.og_parent_scope,
            Message::Agent(AgentMessage {
                agent_id: self.og_parent_scope.clone(),
                message: crate::actors::AgentMessageType::InterAgentMessage(
                    crate::actors::InterAgentMessage::Message {
                        message: format_flag_issue_for_review_manager_message(
                            &self.og_scope,
                            &params.issue_summary,
                        ),
                    },
                ),
            }),
        );

        self.broadcast(Message::Exit);

        self.broadcast_finished(
            &tool_call.id,
            ToolCallResult::Ok(format!(
                "Issue flagged for review: {}",
                params.issue_summary
            )),
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flag_issue_deserialize_params_success() {
        let json_input = r#"{
            "issue_summary": "Agent appears to be stuck in a loop trying to access a non-existent file"
        }"#;

        let result: Result<FlagIssueParams, _> = serde_json::from_str(json_input);
        assert!(result.is_ok());

        let params = result.unwrap();
        assert_eq!(
            params.issue_summary,
            "Agent appears to be stuck in a loop trying to access a non-existent file"
        );
    }

    #[test]
    fn test_flag_issue_deserialize_params_failure() {
        let json_input = r#"{}"#;

        let result: Result<FlagIssueParams, _> = serde_json::from_str(json_input);
        assert!(result.is_err());
    }
}

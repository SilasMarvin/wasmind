use crate::actors::{
    Action, Actor, ActorMessage, AgentMessage, Message, ToolCallStatus, ToolCallType,
    ToolCallUpdate,
};
use crate::scope::Scope;
use genai::chat::{Tool, ToolCall};
use serde_json::json;
use tokio::sync::broadcast;

pub fn format_flag_issue_for_review_manager_message(flagged_agent: &Scope, issue: &str) -> String {
    format!(
        r#"Agent: {flagged_agent}\n has been flagged by the system health check. It has been automatically stopped and is waiting for a message from you.\nReason: {issue}"#
    )
}

/// Tool for temporal agents to flag issues for review
pub struct FlagIssueForReview {
    tx: broadcast::Sender<ActorMessage>,
    scope: Scope,
    og_scope: Scope,
    og_parent_scope: Scope,
}

impl FlagIssueForReview {
    const TOOL_NAME: &'static str = "flag_issue_for_review";

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

    pub fn get_tool_schema() -> Tool {
        Tool {
            name: Self::TOOL_NAME.to_string(),
            description: Some(
                "Flags that the analyzed agent appears to be stuck or in a loop.".to_string(),
            ),
            schema: Some(json!({
                "type": "object",
                "properties": {
                    "issue_summary": {
                        "type": "string",
                        "description": "A one-sentence summary of why the agent seems stuck. Example: 'The agent is repeatedly trying to access a file that does not exist.'"
                    }
                },
                "required": ["issue_summary"]
            })),
        }
    }

    pub async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        if tool_call.fn_name != Self::TOOL_NAME {
            return;
        }

        // Broadcast received
        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id.clone(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::FlagIssueForReview,
                friendly_command_display: "Flagging issue for review".to_string(),
            },
        }));

        // Parse input
        let issue_summary = match tool_call.fn_arguments.get("issue_summary") {
            Some(summary) => summary.as_str().unwrap_or("Unknown issue"),
            None => {
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(
                        "Missing issue_summary parameter".to_string()
                    )),
                }));
                return;
            }
        };

        // Broadcast interrupt and wait for manager
        self.broadcast_with_scope(
            &self.og_scope,
            Message::Agent(AgentMessage {
                agent_id: self.og_scope.clone(),
                message: crate::actors::AgentMessageType::InterAgentMessage(
                    crate::actors::InterAgentMessage::InterruptAndForceWaitForManager {
                        tool_call_id: tool_call.call_id.clone(),
                    },
                ),
            }),
        );

        // Broadcast new message to manager
        self.broadcast_with_scope(
            &self.og_parent_scope,
            Message::Agent(AgentMessage {
                agent_id: self.og_parent_scope.clone(),
                message: crate::actors::AgentMessageType::InterAgentMessage(
                    crate::actors::InterAgentMessage::Message {
                        message: format_flag_issue_for_review_manager_message(
                            &self.og_scope,
                            issue_summary,
                        ),
                    },
                ),
            }),
        );

        // Shut everything down as it was fine
        self.broadcast(Message::Action(Action::Exit));

        // Send tool call completion
        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(format!(
                "Issue flagged for review: {}",
                issue_summary
            ))),
        }));
    }
}

#[async_trait::async_trait]
impl Actor for FlagIssueForReview {
    const ACTOR_ID: &'static str = "flag_issue_for_review";

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    fn get_scope(&self) -> &Scope {
        &self.scope
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        match message.message {
            Message::AssistantToolCall(tool_call) => {
                self.handle_tool_call(tool_call).await;
            }
            _ => {}
        }
    }

    async fn on_start(&mut self) {
        self.broadcast(Message::ToolsAvailable(vec![Self::get_tool_schema()]));
    }
}


use genai::chat::{Tool, ToolCall};
use serde::Deserialize;
use tokio::sync::broadcast;
use tracing::info;

use crate::actors::{
    Actor, ActorMessage, AgentMessage, AgentMessageType, InterAgentMessage, Message,
    ToolCallStatus, ToolCallType, ToolCallUpdate,
};
use crate::config::ParsedConfig;
use crate::scope::Scope;

/// Format plan approval tool result message
pub fn format_plan_approval_success(agent_id: &Scope) -> String {
    format!("Plan for agent {} approved", agent_id)
}

/// Format plan rejection tool result message
pub fn format_plan_rejection(agent_id: &Scope, reason: &str) -> String {
    format!("Plan for agent {} rejected: {}", agent_id, reason)
}

pub const APPROVE_TOOL_NAME: &str = "approve_plan";
pub const APPROVE_TOOL_DESCRIPTION: &str = "Approve a plan submitted by a spawned agent";
pub const APPROVE_TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "agent_id": {
            "type": "string",
            "description": "The ID of the agent whose plan is being approved"
        }
    },
    "required": ["agent_id"]
}"#;

pub const REJECT_TOOL_NAME: &str = "reject_plan";
pub const REJECT_TOOL_DESCRIPTION: &str = "Reject a plan submitted by a child agent with feedback";
pub const REJECT_TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "agent_id": {
            "type": "string",
            "description": "The ID of the task whose plan is being rejected"
        },
        "reason": {
            "type": "string",
            "description": "The reason for rejecting the plan"
        }
    },
    "required": ["agent_id", "reason"]
}"#;

#[derive(Debug, Deserialize)]
struct ApprovePlanInput {
    agent_id: Scope,
}

#[derive(Debug, Deserialize)]
struct RejectPlanInput {
    agent_id: Scope,
    reason: String,
}

/// PlanApproval tool actor for managers to approve/reject plans
pub struct PlanApproval {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)] // TODO: Use for approval timeout, channel buffer sizes
    config: ParsedConfig,
    scope: Scope,
}

impl PlanApproval {
    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self { tx, config, scope }
    }

    async fn handle_approve_plan(&mut self, tool_call: ToolCall) {
        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id.clone(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::MCP,
                friendly_command_display: "Approving plan".to_string(),
            },
        }));

        let input: ApprovePlanInput = match serde_json::from_value(tool_call.fn_arguments) {
            Ok(input) => input,
            Err(e) => {
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!("Invalid input: {}", e))),
                }));
                return;
            }
        };

        let _ = self.broadcast_with_scope(
            &input.agent_id,
            Message::Agent(AgentMessage {
                agent_id: input.agent_id,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::PlanApproved),
            }),
        );

        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(format_plan_approval_success(&input.agent_id))),
        }));
    }

    async fn handle_reject_plan(&mut self, tool_call: ToolCall) {
        info!("Reject plan tool called with ID: {}", tool_call.call_id);

        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id.clone(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::MCP,
                friendly_command_display: "Rejecting plan".to_string(),
            },
        }));

        let input: RejectPlanInput = match serde_json::from_value(tool_call.fn_arguments) {
            Ok(input) => input,
            Err(e) => {
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!("Invalid input: {}", e))),
                }));
                return;
            }
        };

        let _ = self.broadcast_with_scope(
            &input.agent_id,
            Message::Agent(AgentMessage {
                agent_id: input.agent_id.clone(),
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::PlanRejected {
                    reason: input.reason.clone(),
                }),
            }),
        );

        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(format_plan_rejection(
                &input.agent_id,
                &input.reason,
            ))),
        }));
    }

    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        match tool_call.fn_name.as_str() {
            APPROVE_TOOL_NAME => self.handle_approve_plan(tool_call).await,
            REJECT_TOOL_NAME => self.handle_reject_plan(tool_call).await,
            _ => {}
        }
    }
}

#[async_trait::async_trait]
impl Actor for PlanApproval {
    const ACTOR_ID: &'static str = "plan_approval";

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    fn get_scope(&self) -> &Scope {
        &self.scope
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
        // Send tool availability
        let approve_tool = Tool {
            name: APPROVE_TOOL_NAME.to_string(),
            description: Some(APPROVE_TOOL_DESCRIPTION.to_string()),
            schema: Some(serde_json::from_str(APPROVE_TOOL_INPUT_SCHEMA).unwrap()),
        };

        let reject_tool = Tool {
            name: REJECT_TOOL_NAME.to_string(),
            description: Some(REJECT_TOOL_DESCRIPTION.to_string()),
            schema: Some(serde_json::from_str(REJECT_TOOL_INPUT_SCHEMA).unwrap()),
        };

        let _ = self.broadcast(Message::ToolsAvailable(vec![approve_tool, reject_tool]));
    }
}

use genai::chat::{Tool, ToolCall};
use serde::Deserialize;
use tokio::sync::broadcast;
use tracing::info;

use crate::actors::{
    Actor, Message, ToolCallStatus, ToolCallType, ToolCallUpdate,
    agent::{InterAgentMessage, TaskId},
};
use crate::config::ParsedConfig;

pub const APPROVE_TOOL_NAME: &str = "approve_plan";
pub const APPROVE_TOOL_DESCRIPTION: &str = "Approve a plan submitted by a child agent";
pub const APPROVE_TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "task_id": {
            "type": "string",
            "description": "The ID of the task whose plan is being approved"
        },
        "plan_id": {
            "type": "string",
            "description": "The ID of the plan being approved"
        }
    },
    "required": ["task_id", "plan_id"]
}"#;

pub const REJECT_TOOL_NAME: &str = "reject_plan";
pub const REJECT_TOOL_DESCRIPTION: &str = "Reject a plan submitted by a child agent with feedback";
pub const REJECT_TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "task_id": {
            "type": "string",
            "description": "The ID of the task whose plan is being rejected"
        },
        "plan_id": {
            "type": "string",
            "description": "The ID of the plan being rejected"
        },
        "reason": {
            "type": "string",
            "description": "The reason for rejecting the plan"
        }
    },
    "required": ["task_id", "plan_id", "reason"]
}"#;

#[derive(Debug, Deserialize)]
struct ApprovePlanInput {
    task_id: String,
    plan_id: String,
}

#[derive(Debug, Deserialize)]
struct RejectPlanInput {
    task_id: String,
    plan_id: String,
    reason: String,
}

/// PlanApproval tool actor for managers to approve/reject plans
pub struct PlanApproval {
    tx: broadcast::Sender<Message>,
    #[allow(dead_code)] // TODO: Use for approval timeout, channel buffer sizes
    config: ParsedConfig,
    /// Channel to communicate with child agents
    child_tx: broadcast::Sender<InterAgentMessage>,
}

impl PlanApproval {
    pub fn new_with_channel(
        config: ParsedConfig,
        tx: broadcast::Sender<Message>,
        child_tx: broadcast::Sender<InterAgentMessage>,
    ) -> Self {
        Self {
            tx,
            config,
            child_tx,
        }
    }

    async fn handle_approve_plan(&mut self, tool_call: ToolCall) {
        info!("Approve plan tool called with ID: {}", tool_call.call_id);

        // Send received status
        let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id.clone(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::MCP,
                friendly_command_display: "Approving plan".to_string(),
            },
        }));

        // Parse input
        let input: ApprovePlanInput = match serde_json::from_value(tool_call.fn_arguments) {
            Ok(input) => input,
            Err(e) => {
                let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!("Invalid input: {}", e))),
                }));
                return;
            }
        };

        // Send approval to child
        let _ = self.child_tx.send(InterAgentMessage::PlanApproved {
            task_id: TaskId(input.task_id.clone()),
            plan_id: input.plan_id.clone(),
        });

        let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(format!(
                "Plan {} for task {} approved",
                input.plan_id, input.task_id
            ))),
        }));
    }

    async fn handle_reject_plan(&mut self, tool_call: ToolCall) {
        info!("Reject plan tool called with ID: {}", tool_call.call_id);

        // Send received status
        let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id.clone(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::MCP,
                friendly_command_display: "Rejecting plan".to_string(),
            },
        }));

        // Parse input
        let input: RejectPlanInput = match serde_json::from_value(tool_call.fn_arguments) {
            Ok(input) => input,
            Err(e) => {
                let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!("Invalid input: {}", e))),
                }));
                return;
            }
        };

        // Send rejection to child
        let _ = self.child_tx.send(InterAgentMessage::PlanRejected {
            task_id: TaskId(input.task_id.clone()),
            plan_id: input.plan_id.clone(),
            reason: input.reason.clone(),
        });

        let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(format!(
                "Plan {} for task {} rejected: {}",
                input.plan_id, input.task_id, input.reason
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

    fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self {
        // This shouldn't be called directly, use new_with_channel instead
        let (child_tx, _) = broadcast::channel(1024);
        Self {
            tx,
            config,
            child_tx,
        }
    }

    fn get_rx(&self) -> broadcast::Receiver<Message> {
        self.tx.subscribe()
    }

    fn get_tx(&self) -> broadcast::Sender<Message> {
        self.tx.clone()
    }

    async fn handle_message(&mut self, message: Message) {
        match message {
            Message::AssistantToolCall(tool_call) => {
                self.handle_tool_call(tool_call).await;
            }
            _ => {}
        }
    }

    async fn on_start(&mut self) {
        info!("PlanApproval tool actor started");

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

        let _ = self
            .tx
            .send(Message::ToolsAvailable(vec![approve_tool, reject_tool]));
    }
}

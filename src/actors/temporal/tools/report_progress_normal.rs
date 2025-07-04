use crate::actors::{
    Action, Actor, ActorMessage, Message, ToolCallStatus, ToolCallType, ToolCallUpdate,
};
use crate::scope::Scope;
use genai::chat::{Tool, ToolCall};
use serde_json::json;
use tokio::sync::broadcast;

/// Tool for temporal agents to report normal progress
pub struct ReportProgressNormal {
    tx: broadcast::Sender<ActorMessage>,
    scope: Scope,
}

impl ReportProgressNormal {
    const TOOL_NAME: &'static str = "report_progress_normal";

    pub fn new(tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self { tx, scope }
    }

    pub fn get_tool_schema() -> Tool {
        Tool {
            name: Self::TOOL_NAME.to_string(),
            description: Some(
                "Report that the analyzed agent is healthy and making normal progress.".to_string(),
            ),
            schema: Some(json!({
                "type": "object",
                "properties": {},
                "required": []
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
                r#type: ToolCallType::ReportProgressNormal,
                friendly_command_display: "Reporting normal progress".to_string(),
            },
        }));

        // Shut everything down as it was fine
        self.broadcast(Message::Action(Action::Exit));

        // Send tool call completion
        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok("Agent progress reported as normal".to_string())),
        }));
    }
}

#[async_trait::async_trait]
impl Actor for ReportProgressNormal {
    const ACTOR_ID: &'static str = "report_progress_normal";

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


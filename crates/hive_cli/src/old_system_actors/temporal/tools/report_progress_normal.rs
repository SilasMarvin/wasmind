use crate::actors::{ActorContext, ActorMessage, Message, ToolCallResult};
use crate::llm_client::ToolCall;
use crate::scope::Scope;
use tokio::sync::broadcast;

use crate::actors::tools::Tool;

/// Tool for temporal agents to report normal progress
#[derive(hive_macros::ActorContext)]
pub struct ReportProgressNormal {
    tx: broadcast::Sender<ActorMessage>,
    scope: Scope,
}

impl ReportProgressNormal {
    pub fn new(tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self { tx, scope }
    }
}

#[async_trait::async_trait]
impl Tool for ReportProgressNormal {
    const TOOL_NAME: &str = "report_progress_normal";
    const TOOL_DESCRIPTION: &str =
        "Report that the analyzed agent is healthy and making normal progress.";
    const TOOL_INPUT_SCHEMA: &str = r#"{
        "type": "object",
        "properties": {},
        "required": []
    }"#;

    type Params = serde_json::Value;

    async fn execute_tool_call(&mut self, tool_call: ToolCall, _params: Self::Params) {
        self.broadcast(Message::Exit);

        self.broadcast_finished(
            &tool_call.id,
            ToolCallResult::Ok("Agent progress reported as normal".to_string()),
            None,
        );
    }
}

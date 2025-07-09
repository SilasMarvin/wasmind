use crate::actors::{
    ActorContext, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, InterAgentMessage,
    Message, ToolCallStatus, ToolCallUpdate, ToolDisplayInfo, WaitReason,
};
use crate::config::ParsedConfig;
use crate::llm_client;
use crate::scope::Scope;
use tokio::sync::broadcast;

use super::Tool;

pub const WAIT_TOOL_RESPONSE: &str = "Waiting...";

/// Wait tool actor for managers to wait X seconds
#[derive(hive_macros::ActorContext)]
pub struct WaitTool {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)]
    config: ParsedConfig,
    scope: Scope,
}

impl WaitTool {
    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self { config, tx, scope }
    }
}

#[async_trait::async_trait]
impl Tool for WaitTool {
    const TOOL_NAME: &str = "wait";
    const TOOL_DESCRIPTION: &str = "Pause and wait for a new system / sub agent message.";
    const TOOL_INPUT_SCHEMA: &str = r#"{
        "type": "object",
        "required": []
    }"#;

    // This is unused for the Wait tool so just set it to something that won't fail deserialization
    type Params = serde_json::Value;

    async fn execute_tool_call(&mut self, tool_call: llm_client::ToolCall, _params: Self::Params) {
        self.broadcast(Message::Agent(AgentMessage {
            agent_id: self.get_scope().clone(),
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdateRequest {
                tool_call_id: tool_call.id.clone(),
                status: AgentStatus::Wait {
                    reason: WaitReason::WaitForSystem {
                        tool_name: Some(Self::TOOL_NAME.to_string()),
                        tool_call_id: tool_call.id.clone(),
                    },
                },
            }),
        }));

        self.broadcast_finished(
            &tool_call.id,
            Ok(WAIT_TOOL_RESPONSE.to_string()),
            Some(ToolDisplayInfo {
                collapsed: "Waiting...".to_string(),
                expanded: None,
            }),
        );
    }
}

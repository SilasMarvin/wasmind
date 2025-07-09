use crate::actors::{
    Actor, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, InterAgentMessage, Message,
    ToolCallStatus, ToolCallUpdate, WaitReason,
};
use crate::config::ParsedConfig;
use crate::llm_client;
use crate::scope::Scope;
use tokio::sync::broadcast;

use super::Tool;

pub const WAIT_TOOL_RESPONSE: &str = "Waiting...";

/// Wait tool actor for managers to wait X seconds
pub struct Wait {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)]
    config: ParsedConfig,
    scope: Scope,
}

impl Wait {
    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self { config, tx, scope }
    }

    // async fn handle_wait(&mut self, tool_call: ToolCall) {
    //     // TODO: Broadcast received
    //
    //     // Send agent status update first to stop LLM processing
    //     self.broadcast(Message::Agent(AgentMessage {
    //         agent_id: self.get_scope().clone(),
    //         message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdateRequest {
    //             tool_call_id: tool_call.id.clone(),
    //             status: AgentStatus::Wait {
    //                 reason: WaitReason::WaitForSystem {
    //                     tool_name: Some(WAIT_TOOL_NAME.to_string()),
    //                     tool_call_id: tool_call.id.clone(),
    //                 },
    //             },
    //         }),
    //     }));
    //
    //     self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
    //         call_id: tool_call.id,
    //         status: ToolCallStatus::Finished {
    //             result: Ok(WAIT_TOOL_RESPONSE.to_string()),
    //             tui_display: None
    //         },
    //     }));
    // }
    //
    // async fn handle_tool_call(&mut self, tool_call: ToolCall) {
    //     match tool_call.function.name.as_str() {
    //         WAIT_TOOL_NAME => self.handle_wait(tool_call).await,
    //         _ => {}
    //     }
    // }
}

impl 

#[async_trait::async_trait]
impl Tool for Wait {
    const TOOL_NAME: &str = "wait";
    const TOOL_DESCRIPTION: &str = "Pause and wait for a new system / sub agent message.";
    const TOOL_INPUT_SCHEMA: &str = r#"{
        "type": "object",
        "required": []
    }"#;

    // This is unused for the Wait tool so just set it to something that won't fail deserialization
    type Params = serde_json::Value;

    fn get_scope(&self) -> &Scope {
        &self.scope
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    async fn execute_tool_call(&mut self, tool_call: llm_client::ToolCall, params: Self::Params) {
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

        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.id,
            status: ToolCallStatus::Finished {
                result: Ok(WAIT_TOOL_RESPONSE.to_string()),
                tui_display: None,
            },
        }));
    }
}

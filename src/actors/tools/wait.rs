use crate::actors::{
    Actor, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, InterAgentMessage, Message,
    ToolCallStatus, ToolCallUpdate, WaitReason,
};
use crate::config::ParsedConfig;
use crate::scope::Scope;
use genai::chat::{Tool, ToolCall};
use tokio::sync::broadcast;

pub const WAIT_TOOL_RESPONSE: &str = "Waiting...";

pub const WAIT_TOOL_NAME: &str = "wait";
pub const WAIT_TOOL_DESCRIPTION: &str = "Pause and wait for a new system / sub agent message.";
pub const WAIT_TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "required": []
}"#;

/// Wait tool actor for managers to wait X seconds
pub struct Wait {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)] // TODO
    config: ParsedConfig,
    scope: Scope,
}

impl Wait {
    pub const ACTOR_ID: &'static str = "wait";

    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self { config, tx, scope }
    }

    pub fn get_tool_schema() -> Tool {
        Tool {
            name: WAIT_TOOL_NAME.to_string(),
            description: Some(WAIT_TOOL_DESCRIPTION.to_string()),
            schema: Some(
                serde_json::from_str(WAIT_TOOL_INPUT_SCHEMA)
                    .expect("Invalid WAIT_TOOL_INPUT_SCHEMA"),
            ),
        }
    }

    async fn handle_wait(&mut self, tool_call: ToolCall) {
        // Send agent status update first to stop LLM processing
        self.broadcast(Message::Agent(AgentMessage {
            agent_id: self.get_scope().clone(),
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdateRequest {
                tool_call_id: tool_call.call_id.clone(),
                status: AgentStatus::Wait {
                    reason: WaitReason::WaitForSystem {
                        tool_call_id: tool_call.call_id.clone(),
                    },
                },
            }),
        }));

        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(WAIT_TOOL_RESPONSE.to_string())),
        }));
    }

    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        match tool_call.fn_name.as_str() {
            WAIT_TOOL_NAME => self.handle_wait(tool_call).await,
            _ => {}
        }
    }
}

#[async_trait::async_trait]
impl Actor for Wait {
    const ACTOR_ID: &'static str = "wait";

    fn get_scope(&self) -> &Scope {
        &self.scope
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        match message.message {
            Message::AssistantToolCall(tool_call) if message.scope == self.scope => {
                self.handle_tool_call(tool_call).await;
            }
            _ => {}
        }
    }

    async fn on_start(&mut self) {
        let _ = self.tx.send(ActorMessage {
            scope: self.scope,
            message: Message::ToolsAvailable(vec![Self::get_tool_schema()]),
        });
    }
}

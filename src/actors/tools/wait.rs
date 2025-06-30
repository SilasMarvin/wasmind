use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::actors::{
    Actor, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, InterAgentMessage, Message,
    ToolCallStatus, ToolCallUpdate, WaitReason,
};
use crate::config::ParsedConfig;
use crate::scope::Scope;
use genai::chat::{Tool, ToolCall};
use serde::Deserialize;
use tokio::sync::broadcast;

pub const WAIT_TOOL_NAME: &str = "wait";
pub const WAIT_TOOL_DESCRIPTION: &str = "Wait for a specified number of seconds. You will be woken up when the duration ends or by important system messages.";
pub const WAIT_TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "duration": {
            "type": "integer",
            "description": "The number of seconds to wait for."
        }
    },
    "required": ["duration"]
}"#;

#[derive(Debug, Deserialize)]
struct WaitInput {
    duration: u64,
}

/// Format send message success message
pub fn format_wait_response(duration: u64) -> String {
    format!("Waited for {duration} seconds")
}

pub fn format_wait_response_interupted(duration: u64, asked: u64) -> String {
    format!("Wait interrupted - waited for {duration}/{asked} seconds")
}

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
        let input: WaitInput = match serde_json::from_value(tool_call.fn_arguments) {
            Ok(input) => input,
            Err(e) => {
                let error_msg = format!("Invalid wait arguments: {}", e);
                let _ = self.tx.send(ActorMessage {
                    scope: self.scope,
                    message: Message::ToolCallUpdate(ToolCallUpdate {
                        call_id: tool_call.call_id,
                        status: ToolCallStatus::Finished(Err(error_msg)),
                    }),
                });
                return;
            }
        };

        // Send agent status update first to stop LLM processing
        let _ = self.broadcast(Message::Agent(AgentMessage {
            agent_id: self.get_scope().clone(),
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                status: AgentStatus::Wait {
                    reason: WaitReason::WaitForDuration {
                        tool_call_id: tool_call.call_id.clone(),
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                        duration: Duration::from_secs(input.duration),
                    },
                },
            }),
        }));

        let local_tx = self.tx.clone();
        let local_scope = self.scope.clone();
        let secs = input.duration;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(secs)).await;
            let _ = local_tx.send(ActorMessage {
                scope: local_scope,
                message: Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Ok(format_wait_response(input.duration))),
                }),
            });
        });
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
    const ACTOR_ID: &'static str = "send_message";

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

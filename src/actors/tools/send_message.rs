use crate::actors::{
    Actor, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, InterAgentMessage, Message,
    ToolCallStatus, ToolCallUpdate, WaitReason,
};
use crate::config::ParsedConfig;
use crate::scope::Scope;
use genai::chat::{Tool, ToolCall};
use serde::Deserialize;
use tokio::sync::broadcast;

pub const SEND_MESSAGE_TOOL_NAME: &str = "send_message";
pub const SEND_MESSAGE_TOOL_DESCRIPTION: &str = "Send a message to a subordinate agent";
pub const SEND_MESSAGE_TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "agent_id": {
            "type": "string",
            "description": "The ID of the agent to send the message to"
        },
        "message": {
            "type": "string",
            "description": "The message to send"
        },
        "wait": {
            "type": "bool",
            "description": "If `true` pause and wait for a response else continue performing actions (default `false`)"
        }
    },
    "required": ["agent_id", "message"]
}"#;

#[derive(Debug, Deserialize)]
struct SendMessageInput {
    agent_id: String,
    message: String,
    wait: Option<bool>,
}

/// Format send message success message
pub fn format_send_message_success(agent_id: &str) -> String {
    format!("Message sent to agent {agent_id} - please allow at least 5 minutes for a response.")
}

/// SendMessage tool actor for managers to send messages to subordinate agents
pub struct SendMessage {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)] // TODO: Use for timeout configuration
    config: ParsedConfig,
    scope: Scope,
}

impl SendMessage {
    pub const ACTOR_ID: &'static str = "send_message";

    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self { config, tx, scope }
    }

    pub fn get_tool_schema() -> Tool {
        Tool {
            name: SEND_MESSAGE_TOOL_NAME.to_string(),
            description: Some(SEND_MESSAGE_TOOL_DESCRIPTION.to_string()),
            schema: Some(
                serde_json::from_str(SEND_MESSAGE_TOOL_INPUT_SCHEMA)
                    .expect("Invalid SEND_MESSAGE_TOOL_INPUT_SCHEMA"),
            ),
        }
    }

    async fn handle_send_message(&mut self, tool_call: ToolCall) {
        let input: SendMessageInput = match serde_json::from_value(tool_call.fn_arguments) {
            Ok(input) => input,
            Err(e) => {
                let error_msg = format!("Invalid send_message arguments: {}", e);
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

        // Parse the agent ID
        let agent_scope = match input.agent_id.parse::<uuid::Uuid>() {
            Ok(uuid) => Scope::from_uuid(uuid),
            Err(e) => {
                let error_msg = format!("Invalid agent ID format: {}", e);
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

        // Send the message
        self.broadcast(Message::Agent(AgentMessage {
            agent_id: agent_scope,
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::Message {
                message: input.message,
            }),
        }));

        // Maybe broadcast a request to wait
        if input.wait.unwrap_or_default() {
            self.broadcast(Message::Agent(AgentMessage {
                agent_id: self.scope.clone(),
                message: AgentMessageType::InterAgentMessage(
                    InterAgentMessage::StatusUpdateRequest {
                        tool_call_id: tool_call.call_id.clone(),
                        status: AgentStatus::Wait {
                            reason: WaitReason::WaitForSystem {
                                tool_call_id: tool_call.call_id.clone(),
                            },
                        },
                    },
                ),
            }));
        }

        // Send tool success response
        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(format_send_message_success(&input.agent_id))),
        }));
    }

    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        match tool_call.fn_name.as_str() {
            SEND_MESSAGE_TOOL_NAME => self.handle_send_message(tool_call).await,
            _ => {}
        }
    }
}

#[async_trait::async_trait]
impl Actor for SendMessage {
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

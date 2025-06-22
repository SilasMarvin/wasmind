use crate::actors::{
    Actor, ActorMessage, AgentMessage, AgentMessageType, InterAgentMessage, Message,
    ToolCallStatus, ToolCallUpdate, WaitReason, AgentStatus,
};
use crate::config::ParsedConfig;
use crate::scope::Scope;
use genai::chat::{Tool, ToolCall};
use serde::Deserialize;
use tokio::sync::broadcast;

pub const SEND_MANAGER_MESSAGE_TOOL_NAME: &str = "send_manager_message";
pub const SEND_MANAGER_MESSAGE_TOOL_DESCRIPTION: &str = "Send a message to your manager";
pub const SEND_MANAGER_MESSAGE_TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "message": {
            "type": "string",
            "description": "The message to send to your manager"
        },
        "wait": {
            "type": "boolean",
            "description": "Whether to wait for a response from the manager"
        }
    },
    "required": ["message", "wait"]
}"#;

#[derive(Debug, Deserialize)]
struct SendManagerMessageInput {
    message: String,
    wait: bool,
}

/// Format send manager message success message
pub fn format_send_manager_message_success(waiting: bool) -> String {
    if waiting {
        "Message sent to manager - waiting for response".to_string()
    } else {
        "Message sent to manager".to_string()
    }
}

/// SendManagerMessage tool actor for agents to send messages to their manager
pub struct SendManagerMessage {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)] // TODO: Use for timeout configuration
    config: ParsedConfig,
    scope: Scope,
    parent_scope: Scope,
}

impl SendManagerMessage {
    pub const ACTOR_ID: &'static str = "send_manager_message";

    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope, parent_scope: Scope) -> Self {
        Self { config, tx, scope, parent_scope }
    }

    pub fn get_tool_schema() -> Tool {
        Tool {
            name: SEND_MANAGER_MESSAGE_TOOL_NAME.to_string(),
            description: Some(SEND_MANAGER_MESSAGE_TOOL_DESCRIPTION.to_string()),
            schema: Some(
                serde_json::from_str(SEND_MANAGER_MESSAGE_TOOL_INPUT_SCHEMA)
                    .expect("Invalid SEND_MANAGER_MESSAGE_TOOL_INPUT_SCHEMA"),
            ),
        }
    }

    async fn handle_send_manager_message(&mut self, tool_call: ToolCall) {
        let input: SendManagerMessageInput = match serde_json::from_value(tool_call.fn_arguments) {
            Ok(input) => input,
            Err(e) => {
                let error_msg = format!("Invalid send_manager_message arguments: {}", e);
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

        // Send the SubAgentMessage to the parent manager
        let _ = self.broadcast(Message::Agent(AgentMessage {
            agent_id: self.parent_scope,
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::SubAgentMessage {
                message: input.message,
            }),
        }));

        if input.wait {
            // Set agent to wait state for manager response
            let _ = self.broadcast(Message::Agent(AgentMessage {
                agent_id: self.scope,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status: AgentStatus::Wait {
                        tool_call_id: tool_call.call_id.clone(),
                        reason: WaitReason::WaitingForManagerResponse,
                    },
                }),
            }));
        }

        // Send tool success response
        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(format_send_manager_message_success(input.wait))),
        }));
    }

    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        match tool_call.fn_name.as_str() {
            SEND_MANAGER_MESSAGE_TOOL_NAME => self.handle_send_manager_message(tool_call).await,
            _ => {}
        }
    }
}

#[async_trait::async_trait]
impl Actor for SendManagerMessage {
    const ACTOR_ID: &'static str = "send_manager_message";

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
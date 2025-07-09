use crate::actors::{
    Actor, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, InterAgentMessage, Message,
    ToolCallStatus, ToolCallUpdate, WaitReason,
};
use crate::config::ParsedConfig;
use crate::llm_client::{Tool, ToolCall};
use crate::scope::Scope;
use serde::Deserialize;
use tokio::sync::broadcast;

pub const SEND_MANAGER_MESSAGE_SUCCESS_TOOL_RESPONSE: &'static str =
    "Message sent to manager. Expect a response in 300 seconds.";

pub const SEND_MANAGER_MESSAGE_TOOL_NAME: &str = "send_manager_message";
pub const SEND_MANAGER_MESSAGE_TOOL_DESCRIPTION: &str = "Send a message to your manager";
pub const SEND_MANAGER_MESSAGE_TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "message": {
            "type": "string",
            "description": "The message to send to your manager"
        }
    },
    "wait": {
        "type": "boolean",
        "description": "If `true` pause and wait for a response from your manager else continue performing actions (default `false`)"
    },
    "required": ["message"]
}"#;

#[derive(Debug, Deserialize)]
struct SendManagerMessageInput {
    message: String,
    wait: Option<bool>,
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

    pub fn new(
        config: ParsedConfig,
        tx: broadcast::Sender<ActorMessage>,
        scope: Scope,
        parent_scope: Scope,
    ) -> Self {
        Self {
            config,
            tx,
            scope,
            parent_scope,
        }
    }

    pub fn get_tool_schema() -> Tool {
        Tool {
            tool_type: "function".to_string(),
            function: crate::llm_client::ToolFunction {
                name: SEND_MANAGER_MESSAGE_TOOL_NAME.to_string(),
                description: SEND_MANAGER_MESSAGE_TOOL_DESCRIPTION.to_string(),
                parameters: serde_json::from_str(SEND_MANAGER_MESSAGE_TOOL_INPUT_SCHEMA)
                    .expect("Invalid SEND_MANAGER_MESSAGE_TOOL_INPUT_SCHEMA"),
            },
        }
    }

    async fn handle_send_manager_message(&mut self, tool_call: ToolCall) {
        let input: SendManagerMessageInput =
            match serde_json::from_str(&tool_call.function.arguments) {
                Ok(input) => input,
                Err(e) => {
                    let error_msg = format!("Invalid send_manager_message arguments: {}", e);
                    let _ = self.tx.send(ActorMessage {
                        scope: self.scope,
                        message: Message::ToolCallUpdate(ToolCallUpdate {
                            call_id: tool_call.id,
                            status: ToolCallStatus::Finished { 
                                result: Err(error_msg), 
                                tui_display: None 
                            },
                        }),
                    });
                    return;
                }
            };

        // Send the SubAgentMessage to the parent manager
        self.broadcast(Message::Agent(AgentMessage {
            agent_id: self.parent_scope,
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
                        tool_call_id: tool_call.id.clone(),
                        status: AgentStatus::Wait {
                            reason: WaitReason::WaitingForManager {
                                tool_name: Some(SEND_MANAGER_MESSAGE_TOOL_NAME.to_string()),
                                tool_call_id: tool_call.id.clone(),
                            },
                        },
                    },
                ),
            }));
        }

        // Send tool success response
        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.id,
            status: ToolCallStatus::Finished {
                result: Ok(SEND_MANAGER_MESSAGE_SUCCESS_TOOL_RESPONSE.to_string()),
                tui_display: None,
            },
        }));
    }

    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        match tool_call.function.name.as_str() {
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

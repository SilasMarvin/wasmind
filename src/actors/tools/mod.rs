use serde::de::DeserializeOwned;
use tokio::sync::broadcast;

use super::{Action, Actor, ActorMessage, Message, ToolCallResult, ToolCallStatus, ToolCallUpdate};
use crate::{llm_client, scope::Scope};

pub mod command;
pub mod complete;
pub mod edit_file;
pub mod file_reader;
pub mod mcp;
pub mod planner;
pub mod send_manager_message;
pub mod send_message;
pub mod spawn_agent;
pub mod wait;

#[async_trait::async_trait]
pub trait Tool {
    const TOOL_NAME: &str;
    const TOOL_DESCRIPTION: &str;
    const TOOL_INPUT_SCHEMA: &str;

    type Params: DeserializeOwned;

    /// gets the scope
    fn get_scope(&self) -> &Scope;

    /// Gets the message sender
    fn get_tx(&self) -> broadcast::Sender<ActorMessage>;

    /// Gets the message receiver
    fn get_rx(&self) -> broadcast::Receiver<ActorMessage>;

    fn handle_tool_call(&mut self, tool_call: llm_client::ToolCall) {
        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.id.clone(),
            status: ToolCallStatus::Received,
        }));

        let params: Self::Params = match serde_json::from_str(&tool_call.function.arguments) {
            Ok(params) => params,
            Err(e) => {
                let error_message = format!("Invalid parameters for tool {}: {e}", Self::TOOL_NAME);
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.id.clone(),
                    status: ToolCallStatus::Finished {
                        result: ToolCallResult::Err(error_message.clone()),
                        tui_display: Some(super::ToolDisplayInfo {
                            collapsed: "Invalid parameters".to_string(),
                            expanded: Some(error_message),
                        }),
                    },
                }));
                return;
            }
        };

        self.execute_tool_call(params);
    }

    fn execute_tool_call(&mut self, params: Self::Params);

    fn awaiting_user_confirmation(&self) -> Option<&str> {
        None
    }

    fn handle_user_confirmed(&mut self) {}

    fn handle_user_denied(&mut self) {}

    fn handle_cancel(&mut self) {}

    /// Sends a message
    fn broadcast(&self, message: Message) {
        let _ = self.get_tx().send(ActorMessage {
            scope: *self.get_scope(),
            message,
        });
    }

    /// Sends a message with a specific scope
    fn broadcast_with_scope(&self, scope: &Scope, message: Message) {
        let _ = self.get_tx().send(ActorMessage {
            scope: *scope,
            message,
        });
    }
}

#[async_trait::async_trait]
impl<T: Tool + Send + 'static> Actor for T {
    const ACTOR_ID: &str = Self::TOOL_NAME;

    fn get_scope(&self) -> &Scope {
        self.get_scope()
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.get_tx()
    }

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.get_rx()
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        match message.message {
            Message::Action(Action::Cancel) => self.handle_cancel(),
            Message::AssistantToolCall(tool_call) if tool_call.function.name == Self::TOOL_NAME => {
                self.handle_tool_call(tool_call);
            }
            Message::ToolCallUpdate(update) => match update.status {
                crate::actors::ToolCallStatus::ReceivedUserYNConfirmation(confirmation) => {
                    if self
                        .awaiting_user_confirmation()
                        .is_some_and(|call_id| call_id == update.call_id)
                    {
                        if confirmation {
                            self.handle_user_confirmed();
                        } else {
                            self.handle_user_denied();
                        }
                    }
                }
                _ => (),
            },
            _ => (),
        }
    }

    async fn on_start(&mut self) {
        let tool = llm_client::Tool {
            tool_type: "function".to_string(),
            function: crate::llm_client::ToolFunction {
                name: Self::TOOL_NAME.to_string(),
                description: Self::TOOL_DESCRIPTION.to_string(),
                parameters: serde_json::from_str(Self::TOOL_INPUT_SCHEMA).unwrap(),
            },
        };
        self.broadcast(Message::ToolsAvailable(vec![tool]));
    }
}

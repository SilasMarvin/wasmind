use serde::de::DeserializeOwned;
use tokio::sync::broadcast;

use super::{
    Action, Actor, ActorContext, ActorMessage, Message, ToolCallResult, ToolCallStatus,
    ToolCallUpdate, ToolDisplayInfo,
};
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
pub trait Tool: ActorContext {
    const TOOL_NAME: &str;
    const TOOL_DESCRIPTION: &str;
    const TOOL_INPUT_SCHEMA: &str;

    type Params: DeserializeOwned;

    async fn handle_tool_call(&mut self, tool_call: llm_client::ToolCall) {
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

        self.execute_tool_call(tool_call, params).await;
    }

    fn awaiting_user_confirmation(&self) -> Option<&str> {
        None
    }

    async fn execute_tool_call(&mut self, tool_call: llm_client::ToolCall, params: Self::Params);

    async fn handle_user_confirmed(&mut self) {}

    async fn handle_user_denied(&mut self) {}

    async fn handle_cancel(&mut self) {}

    /// Broadcasts ToolCallStatus::Finished
    fn broadcast_finished(
        &self,
        tool_call_id: &str,
        result: ToolCallResult,
        tui_display: Option<ToolDisplayInfo>,
    ) {
        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status: ToolCallStatus::Finished {
                result: result,
                tui_display,
            },
        }));
    }
}

#[async_trait::async_trait]
impl<T: Tool + Send + 'static> Actor for T {
    const ACTOR_ID: &str = Self::TOOL_NAME;

    async fn handle_message(&mut self, message: ActorMessage) {
        match message.message {
            Message::Action(Action::Cancel) => self.handle_cancel().await,
            Message::AssistantToolCall(tool_call) if tool_call.function.name == Self::TOOL_NAME => {
                self.handle_tool_call(tool_call).await;
            }
            Message::ToolCallUpdate(update) => match update.status {
                crate::actors::ToolCallStatus::ReceivedUserYNConfirmation(confirmation) => {
                    if self
                        .awaiting_user_confirmation()
                        .is_some_and(|call_id| call_id == update.call_id)
                    {
                        if confirmation {
                            self.handle_user_confirmed().await;
                        } else {
                            self.handle_user_denied().await;
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

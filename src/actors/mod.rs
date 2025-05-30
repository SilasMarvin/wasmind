pub mod tools;
pub mod tui;

use genai::chat::ToolCall;
use std::fmt::Debug;
use tokio::sync::broadcast;

use crate::config::ParsedConfig;

/// UserActions the worker can perform and users can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserAction {
    CaptureWindow,
    CaptureClipboard,
    ToggleRecordMicrophone,
    Assist,
    CancelAssist,
    Exit,
}

/// ToolCall Update
pub struct ToolCallUpdate {
    call_id: String,
    status: ToolCallStatus,
}

/// ToolCall Type
pub enum ToolCallType {
    Command,
    ReadFile,
    EditFile,
    MCP,
}

/// ToolCall Status
pub enum ToolCallStatus {
    Received {
        r#type: ToolCallType,
        friendly_command_display: String,
    },
    AwaitingUserYNConfirmation,
    ReceivedUserYNConfirmation(bool),
    Finished(Result<String, String>),
}

/// The various messages actors can send
#[derive(Debug, Clone)]
pub enum Message {
    UserAction(UserAction),
    AssistantToolCall(ToolCall),
    ToolCallUpdate(ToolCallUpdate),
}

/// Base trait for all actors in the system
#[async_trait::async_trait]
pub trait Actor: Send + Sized + 'static {
    /// new
    fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self;

    /// gets the rx
    fn get_rx(&self) -> broadcast::Receiver<Message>;

    /// run
    fn run(mut self) {
        tokio::spawn(async move {
            self.on_start().await;

            // If this errors we just crash
            while let Ok(msg) = self.get_rx().recv().await {
                self.handle_message(msg).await;
            }

            self.on_stop().await;
        });
    }

    /// Called when a message is broadcasted
    async fn handle_message(&mut self, message: Message);

    /// Called when the actor starts
    async fn on_start(&mut self) {}

    /// Called when the actor stops
    async fn on_stop(&mut self) {}
}

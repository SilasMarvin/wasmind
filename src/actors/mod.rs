pub mod assistant;
pub mod context;
pub mod microphone;
pub mod tools;
pub mod tui;

use genai::chat::ToolCall;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::path::PathBuf;
use tokio::sync::broadcast;
use tracing::error;

use crate::config::ParsedConfig;

/// Actions the worker can perform and users can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    CaptureWindow,
    CaptureClipboard,
    ToggleRecordMicrophone,
    Assist,
    Cancel,
    Exit,
}

impl Action {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "CaptureWindow" => Some(Action::CaptureWindow),
            "CaptureClipboard" => Some(Action::CaptureClipboard),
            "ToggleRecordMicrophone" => Some(Action::ToggleRecordMicrophone),
            "Assist" => Some(Action::Assist),
            "CancelAssist" => Some(Action::Cancel),
            "Exit" => Some(Action::Exit),
            _ => None,
        }
    }
}

/// ToolCall Update
#[derive(Debug, Clone)]
pub struct ToolCallUpdate {
    pub call_id: String,
    pub status: ToolCallStatus,
}

/// ToolCall Type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolCallType {
    Command,
    ReadFile,
    EditFile,
    Planner,
    MCP,
}

/// ToolCall Status
#[derive(Debug, Clone)]
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
    // User actions from keyboard/TUI
    Action(Action),
    UserTUIInput(String),

    // Assistant messages
    AssistantToolCall(ToolCall),
    AssistantResponse(genai::chat::MessageContent),

    // Tool messages
    ToolCallUpdate(ToolCallUpdate),
    ToolsAvailable(Vec<genai::chat::Tool>),

    // Microphone messages
    MicrophoneToggle,
    MicrophoneTranscription(String),

    // TUI messages
    TUIClearInput,

    // Context messages
    ScreenshotCaptured(Result<String, String>), // Ok(base64) or Err(error message)
    ClipboardCaptured(Result<String, String>),  // Ok(text) or Err(error message)

    // System state update messages
    FileRead {
        path: PathBuf,
        content: String,
        last_modified: std::time::SystemTime,
    },
    FileEdited {
        path: PathBuf,
        content: String,
        last_modified: std::time::SystemTime,
    },
    PlanUpdated(crate::actors::tools::planner::TaskPlan),

    // Actor lifecycle messages
    ActorReady {
        actor_id: &'static str,
    },
}

/// Base trait for all actors in the system
#[async_trait::async_trait]
pub trait Actor: Send + Sized + 'static {
    /// Unique identifier for this actor type
    const ACTOR_ID: &'static str;

    /// new
    fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self;

    /// gets the rx
    fn get_rx(&self) -> broadcast::Receiver<Message>;

    /// run
    fn run(mut self) {
        let tx = self.get_tx();
        tokio::spawn(async move {
            self.on_start().await;

            // Signal that this actor is ready
            let _ = tx.send(Message::ActorReady {
                actor_id: Self::ACTOR_ID,
            });

            let mut rx = self.get_rx();
            loop {
                match rx.recv().await {
                    Ok(Message::Action(Action::Exit)) => break,
                    Ok(msg) => self.handle_message(msg).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        error!("RECEIVER LAGGED BY {} MESSAGES! This was unexpected.", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        error!("Channel closed")
                    }
                }
            }

            self.on_stop().await;
        });
    }

    /// Gets the message sender
    fn get_tx(&self) -> broadcast::Sender<Message>;

    /// Called when a message is broadcasted
    async fn handle_message(&mut self, message: Message);

    /// Called when the actor starts
    async fn on_start(&mut self) {}

    /// Called when the actor stops
    async fn on_stop(&mut self) {}
}

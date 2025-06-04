pub mod agent;
pub mod assistant;
#[cfg(feature = "gui")]
pub mod context;
#[cfg(feature = "audio")]
pub mod microphone;
pub mod state_system;
pub mod tools;
pub mod tui;

use genai::chat::ToolCall;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::path::PathBuf;
use tokio::sync::broadcast;

use self::agent::{AgentId, TaskId, TaskStatus};
use crate::config::ParsedConfig;

/// Actions the worker can perform and users can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    #[cfg(feature = "gui")]
    CaptureWindow,
    #[cfg(feature = "gui")]
    CaptureClipboard,
    #[cfg(feature = "audio")]
    ToggleRecordMicrophone,
    Assist,
    Cancel,
    Exit,
}

impl Action {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            #[cfg(feature = "gui")]
            "CaptureWindow" => Some(Action::CaptureWindow),
            #[cfg(feature = "gui")]
            "CaptureClipboard" => Some(Action::CaptureClipboard),
            #[cfg(feature = "audio")]
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
    #[cfg(feature = "audio")]
    MicrophoneToggle,
    #[cfg(feature = "audio")]
    MicrophoneTranscription(String),

    // TUI messages
    TUIClearInput,

    // Context messages
    #[cfg(feature = "gui")]
    ScreenshotCaptured(Result<String, String>), // Ok(base64) or Err(error message)
    #[cfg(feature = "gui")]
    ClipboardCaptured(Result<String, String>), // Ok(text) or Err(error message)

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

    // Agent management messages
    AgentSpawned {
        agent_id: AgentId,
        agent_role: String,
        task_id: TaskId,
        task_description: String,
    },
    AgentStatusUpdate {
        agent_id: AgentId,
        status: TaskStatus,
    },
    AgentRemoved {
        agent_id: AgentId,
    },

    // Actor lifecycle messages
    ActorReady {
        actor_id: &'static str,
    },

    // Task completion message
    TaskCompleted {
        summary: String,
        success: bool,
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
        let actor_id = Self::ACTOR_ID;
        let span = tracing::info_span!("actor_lifecycle", actor_id = actor_id);
        tokio::spawn(async move {
            let _guard = span.enter();
            self.on_start().await;

            // Signal that this actor is ready
            tracing::info!("Actor ready, sending ready signal");
            let _ = tx.send(Message::ActorReady {
                actor_id: Self::ACTOR_ID,
            });

            let mut rx = self.get_rx();
            loop {
                match rx.recv().await {
                    Ok(Message::Action(Action::Exit)) => {
                        tracing::info!("Actor received exit signal");
                        break;
                    }
                    Ok(msg) => {
                        tracing::debug!("Actor handling message");
                        self.handle_message(msg).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::error!("RECEIVER LAGGED BY {} MESSAGES! This was unexpected.", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::error!("Channel closed");
                    }
                }
            }

            tracing::info!("Actor stopping");
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

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
use uuid::Uuid;

/// Actions users can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    TaskCompleted,
    MCP,
}

/// ToolCall Status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolCallStatus {
    Received {
        r#type: ToolCallType,
        friendly_command_display: String,
    },
    AwaitingUserYNConfirmation,
    ReceivedUserYNConfirmation(bool),
    Finished(Result<String, String>),
}

/// Task awaiting manager decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskAwaitingManager {
    AwaitingPlanApproval(crate::actors::tools::planner::TaskPlan),
    AwaitingMoreInformation(String),
}

/// Task status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentTaskStatus {
    Done(Result<String, String>),
    InProgress,
    AwaitingManager(TaskAwaitingManager),
    Waiting { tool_call_id: String },
}

/// Inter-agent message for communication between agents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InterAgentMessage {
    /// Agent reports task status to manager
    TaskStatusUpdate { status: AgentTaskStatus },
    /// Manager approves a plan
    PlanApproved { plan_id: String },
    /// Manager rejects a plan
    PlanRejected { plan_id: String, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    agent_id: Uuid,
    message: AgentMessageType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentMessageType {
    AgentSpawned {
        agent_role: String,
        task_description: String,
    },
    AgentRemoved,
    InterAgentMessage(InterAgentMessage),
}

/// Context provided by the user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UserContext {
    UserTUIInput(String),
    #[cfg(feature = "audio")]
    MicrophoneTranscription(String),
    #[cfg(feature = "gui")]
    ScreenshotCaptured(Result<String, String>), // Ok(base64) or Err(error message)
    #[cfg(feature = "gui")]
    ClipboardCaptured(Result<String, String>), // Ok(text) or Err(error message)
}

/// The various messages actors can send
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    // User actions from keyboard/TUI
    Action(Action),

    // UserContext
    UserContext(UserContext),

    // Assistant messages
    AssistantToolCall(ToolCall),
    AssistantResponse(genai::chat::MessageContent),

    // Tool messages
    ToolCallUpdate(ToolCallUpdate),
    ToolsAvailable(Vec<genai::chat::Tool>),

    // System state update messages
    FileRead {
        path: PathBuf,
        content: String,
        #[serde(skip, default = "std::time::SystemTime::now")]
        last_modified: std::time::SystemTime,
    },
    FileEdited {
        path: PathBuf,
        content: String,
        #[serde(skip, default = "std::time::SystemTime::now")]
        last_modified: std::time::SystemTime,
    },
    PlanUpdated(crate::actors::tools::planner::TaskPlan),

    // Agent messages
    Agent(AgentMessage),

    // Actor lifecycle messages
    ActorReady {
        actor_id: String,
    },

    // Task completion message
    TaskCompleted {
        summary: String,
        success: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorMessage {
    // The agent scope this message exists in
    pub scope: Uuid,
    pub message: Message,
}

/// Base trait for all actors in the system
#[async_trait::async_trait]
pub trait Actor: Send + Sized + 'static {
    /// Unique identifier for this actor type
    const ACTOR_ID: &'static str;

    /// gets the scope
    fn get_scope(&self) -> &Uuid;

    /// get scope filters
    /// Used in the `run` method to filter out messages that are not in the returned scopes
    /// By default only listen to messages in your current scope
    fn get_scope_filters(&self) -> Vec<&Uuid> {
        vec![self.get_scope()]
    }

    /// Gets the message sender
    fn get_tx(&self) -> broadcast::Sender<ActorMessage>;

    /// gets the message receiver
    fn get_rx(&self) -> broadcast::Receiver<ActorMessage>;

    /// Sends a message
    fn broadcast(&self, message: Message) {
        let _ = self.get_tx().send(ActorMessage {
            scope: self.get_scope().clone(),
            message,
        });
    }

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
            let _ = tx.send(ActorMessage {
                scope: self.get_scope().clone(),
                message: Message::ActorReady {
                    actor_id: Self::ACTOR_ID.to_string(),
                },
            });

            let mut rx = self.get_rx();
            loop {
                match rx.recv().await {
                    Ok(ActorMessage {
                        scope: _,
                        message: Message::Action(Action::Exit),
                    }) => {
                        tracing::info!("Actor received exit signal");
                        break;
                    }
                    Ok(msg) => {
                        tracing::debug!("Actor handling message");
                        let current_scope = self.get_scope();
                        if self
                            .get_scope_filters()
                            .iter()
                            .find(|scope| **scope == current_scope)
                            .is_some()
                        {
                            self.handle_message(msg).await;
                        }
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

    /// Called when a message is broadcasted
    async fn handle_message(&mut self, message: ActorMessage);

    /// Called when the actor starts
    async fn on_start(&mut self) {}

    /// Called when the actor stops
    async fn on_stop(&mut self) {}
}

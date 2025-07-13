pub mod agent;
pub mod assistant;
// #[cfg(feature = "gui")]
// pub mod context;
// #[cfg(feature = "audio")]
// pub mod microphone;
pub mod litellm_manager;
pub mod state_system;
pub mod temporal;
pub mod tools;
pub mod tui;

use crate::llm_client::{self, ToolCall};
use crate::scope::Scope;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;
use std::path::PathBuf;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Pending tool call information including name and result
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingToolCall {
    pub tool_name: String,
    pub result: Option<ToolCallResult>,
}

/// ToolCall Update
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallUpdate {
    pub call_id: String,
    pub status: ToolCallStatus,
}

pub type ToolCallResult = Result<String, String>;

/// Display information for tool calls in the TUI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDisplayInfo {
    pub collapsed: String,
    pub expanded: Option<String>,
}

/// ToolCall Status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolCallStatus {
    Received,
    AwaitingUserYNConfirmation,
    ReceivedUserYNConfirmation(bool),
    Finished {
        result: ToolCallResult,
        tui_display: Option<ToolDisplayInfo>,
    },
}

/// The result of an agent task
pub type AgentTaskResult = Result<AgentTaskResultOk, String>;

/// The result of an agent task
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentTaskResultOk {
    pub summary: String,
    pub success: bool,
}

/// Reason for waiting
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WaitReason {
    WaitingForUserInput,
    WaitForSystem {
        tool_name: Option<String>,
        tool_call_id: String,
    },
    WaitingForManager {
        tool_name: Option<String>,
        tool_call_id: String,
    },
    WaitingForTools {
        tool_calls: HashMap<String, PendingToolCall>,
    },
    WaitingForActors {
        pending_actors: Vec<String>,
    },
}

/// Unified agent status that combines assistant state and task status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentStatus {
    /// Actively processing a user request (making LLM call)
    Processing { id: Uuid },
    /// Waiting for something outside of our control to complete
    Wait { reason: WaitReason },
    /// Agent task is complete
    Done(AgentTaskResult),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum InterAgentMessage {
    /// Agent task status update
    StatusUpdate { status: AgentStatus },
    /// Request to update agent status
    StatusUpdateRequest {
        tool_call_id: String,
        status: AgentStatus,
    },
    /// Interrupt and force wait for manager
    InterruptAndForceWaitForManager { tool_call_id: String },
    /// Message between agents
    Message { message: String },
}

/// Messages between two agents or agents and their tools
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentMessageType {
    AgentSpawned {
        agent_type: AgentType,
        role: String,
        task_description: String,
        tool_call_id: String,
    },
    AgentRemoved,
    InterAgentMessage(InterAgentMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentMessage {
    pub agent_id: Scope,
    pub message: AgentMessageType,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AgentType {
    MainManager,
    SubManager,
    Worker,
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                AgentType::MainManager => "Main Manager",
                AgentType::SubManager => "Sub Manager",
                AgentType::Worker => "Worker",
            }
        )
    }
}

/// Context provided by the user
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UserContext {
    UserTUIInput(String),
    #[cfg(feature = "audio")]
    MicrophoneTranscription(String),
    #[cfg(feature = "gui")]
    ScreenshotCaptured(Result<String, String>), // Ok(base64) or Err(error message)
    #[cfg(feature = "gui")]
    ClipboardCaptured(Result<String, String>), // Ok(text) or Err(error message)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantRequest {
    system: String,
    tools: Vec<llm_client::Tool>,
    messages: Vec<llm_client::ChatMessage>,
}

/// The various messages actors can send
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    // UserContext
    UserContext(UserContext),

    // Cancel the current action in this scope
    Cancel,
    // Shutdown this scope
    Exit,

    AssistantRequest(AssistantRequest),
    AssistantChatUpdated(Vec<llm_client::ChatMessage>),
    AssistantToolCall(ToolCall),
    AssistantResponse {
        id: Uuid,
        message: llm_client::AssistantChatMessage,
    },

    // Tool messages
    ToolCallUpdate(ToolCallUpdate),
    ToolsAvailable(Vec<llm_client::Tool>),

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
}

// Manual implementation of PartialEq and Eq for Message
// Some variants contain external types that don't implement these traits
impl PartialEq for Message {
    fn eq(&self, other: &Self) -> bool {
        // TODO: Fill this out
        match (self, other) {
            _ => false,
        }
    }
}

impl Eq for Message {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorMessage {
    // The agent scope this message exists in
    pub scope: Scope,
    pub message: Message,
}

impl PartialOrd for ActorMessage {
    fn partial_cmp(&self, _other: &Self) -> Option<std::cmp::Ordering> {
        None
    }
}

pub trait ActorContext {
    fn get_scope(&self) -> &Scope;
    fn get_tx(&self) -> broadcast::Sender<ActorMessage>;
    fn get_rx(&self) -> broadcast::Receiver<ActorMessage>;

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

/// Trait for all actors in the system
#[async_trait::async_trait]
pub trait Actor: ActorContext + Send + 'static {
    /// Unique identifier for this actor type
    const ACTOR_ID: &'static str;

    /// get scope filters
    /// Used in the `run` method to filter out messages that are not in the returned scopes
    /// By default only listen to messages in your current scope
    fn get_scope_filters(&self) -> Vec<&Scope> {
        vec![self.get_scope()]
    }

    /// run
    fn run(mut self)
    where
        Self: Sized,
    {
        // It is essential that we subscribe to the tx before entering the tokio task or we may
        // miss messages we rely upon. E.G. Message::ActorReady
        let mut rx = self.get_rx();
        let actor_id = Self::ACTOR_ID;
        tracing::info_span!("actor_lifecycle", actor_id = actor_id).in_scope(move || {
            tokio::spawn(async move {
                self.on_start().await;

                // Signal that this actor is ready
                tracing::info!("Actor ready, sending ready signal");
                self.broadcast(Message::ActorReady {
                    actor_id: Self::ACTOR_ID.to_string(),
                });

                loop {
                    match rx.recv().await {
                        Ok(ActorMessage {
                            scope,
                            message: Message::Exit,
                        }) => {
                            if &scope == self.get_scope() {
                                tracing::info!("Actor received exit signal");
                                break;
                            }
                        }
                        Ok(msg) => {
                            if self
                                .get_scope_filters()
                                .iter()
                                .find(|scope| **scope == &msg.scope)
                                .is_some()
                            {
                                self.handle_message(msg).await;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::error!(
                                "RECEIVER LAGGED BY {n} MESSAGES! This was unexpected.",
                            );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::error!("Channel closed");
                        }
                    }
                }

                tracing::info!("Actor stopping");
                self.on_stop().await;
            });
        });
    }

    /// Called when a message is broadcasted
    async fn handle_message(&mut self, message: ActorMessage);

    /// Called when the actor starts
    async fn on_start(&mut self) {}

    /// Called when the actor stops
    async fn on_stop(&mut self) {}
}

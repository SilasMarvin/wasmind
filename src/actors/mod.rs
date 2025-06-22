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
use crate::scope::Scope;

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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallUpdate {
    pub call_id: String,
    pub status: ToolCallStatus,
}

/// ToolCall Type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolCallType {
    Command,
    ReadFile,
    EditFile,
    Planner,
    TaskCompleted,
    SpawnAgent,
    MCP,
}

/// ToolCall Status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolCallStatus {
    Received {
        r#type: ToolCallType,
        friendly_command_display: String,
    },
    AwaitingUserYNConfirmation,
    ReceivedUserYNConfirmation(bool),
    Finished(Result<String, String>),
}

/// Reason for waiting
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WaitReason {
    WaitingForAgentResponse { agent_id: Scope },
    WaitingForManagerResponse,
    WaitingForPlanApproval,
}

/// The result of an agent task
pub type AgentTaskResult = Result<AgentTaskResultOk, String>;

/// The result of an agent task
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentTaskResultOk {
    pub summary: String,
    pub success: bool,
}

/// Unified agent status that combines assistant state and task status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentStatus {
    /// Waiting for actors to be ready
    AwaitingActors,
    /// Ready to accept requests, has tools available
    Idle,
    /// Actively processing a user request (making LLM call)
    Processing,
    /// Waiting for tool execution results
    AwaitingTools { pending_tool_calls: Vec<String> },
    /// Waiting for next input from user, sub agent, etc...
    /// Does not submit a response to the LLM when the tool call with `tool_call_id` returns a
    /// response. Waits for other input
    Wait { tool_call_id: String, reason: WaitReason },
    /// Agent is working on the task
    InProgress,
    /// Agent task is complete
    Done(AgentTaskResult),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum InterAgentMessage {
    /// Agent reports task status to manager
    TaskStatusUpdate { status: AgentStatus },
    /// Manager approves a plan
    PlanApproved,
    /// Manager rejects a plan
    PlanRejected { reason: String },
    /// Manager sends information to an agent
    ManagerMessage { message: String },
    /// Sub-agent sends message to manager
    SubAgentMessage { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentMessage {
    pub agent_id: Scope,
    pub message: AgentMessageType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentType {
    MainManager,
    SubManager,
    Worker,
}

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
}

// Manual implementation of PartialEq and Eq for Message
// Some variants contain external types that don't implement these traits
impl PartialEq for Message {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Message::Action(a), Message::Action(b)) => a == b,
            (Message::UserContext(a), Message::UserContext(b)) => a == b,
            (Message::ToolCallUpdate(a), Message::ToolCallUpdate(b)) => a == b,
            (Message::Agent(a), Message::Agent(b)) => a == b,
            (Message::ActorReady { actor_id: a }, Message::ActorReady { actor_id: b }) => a == b,
            (Message::PlanUpdated(plan1), Message::PlanUpdated(plan2)) => plan1 == plan2,

            // AssistantToolCall - compare call_id
            (Message::AssistantToolCall(a), Message::AssistantToolCall(b)) => {
                a.call_id == b.call_id
            }

            // AssistantResponse - compare by content type/discriminant
            (Message::AssistantResponse(a), Message::AssistantResponse(b)) => {
                use genai::chat::MessageContent;
                match (a, b) {
                    (MessageContent::Text(t1), MessageContent::Text(t2)) => t1 == t2,
                    (MessageContent::ToolCalls(tc1), MessageContent::ToolCalls(tc2)) => {
                        tc1.len() == tc2.len()
                            && tc1
                                .iter()
                                .zip(tc2.iter())
                                .all(|(t1, t2)| t1.call_id == t2.call_id)
                    }
                    (MessageContent::ToolResponses(tr1), MessageContent::ToolResponses(tr2)) => {
                        tr1.len() == tr2.len()
                            && tr1.iter().zip(tr2.iter()).all(|(r1, r2)| {
                                r1.call_id == r2.call_id && r1.content == r2.content
                            })
                    }
                    (MessageContent::Parts(_), MessageContent::Parts(_)) => false, // Too complex, skip
                    _ => false, // Different variants
                }
            }

            // ToolsAvailable - compare tool names
            (Message::ToolsAvailable(tools1), Message::ToolsAvailable(tools2)) => {
                tools1.len() == tools2.len()
                    && tools1
                        .iter()
                        .zip(tools2.iter())
                        .all(|(t1, t2)| t1.name == t2.name)
            }

            // FileRead and FileEdited - compare path and content, skip SystemTime
            (
                Message::FileRead {
                    path: path1,
                    content: content1,
                    last_modified: _,
                },
                Message::FileRead {
                    path: path2,
                    content: content2,
                    last_modified: _,
                },
            ) => path1 == path2 && content1 == content2,
            (
                Message::FileEdited {
                    path: path1,
                    content: content1,
                    last_modified: _,
                },
                Message::FileEdited {
                    path: path2,
                    content: content2,
                    last_modified: _,
                },
            ) => path1 == path2 && content1 == content2,

            // Different variants are never equal
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

/// Base trait for all actors in the system
#[async_trait::async_trait]
pub trait Actor: Send + Sized + 'static {
    /// Unique identifier for this actor type
    const ACTOR_ID: &'static str;

    /// gets the scope
    fn get_scope(&self) -> &Scope;

    /// get scope filters
    /// Used in the `run` method to filter out messages that are not in the returned scopes
    /// By default only listen to messages in your current scope
    fn get_scope_filters(&self) -> Vec<&Scope> {
        vec![self.get_scope()]
    }

    /// Gets the message sender
    fn get_tx(&self) -> broadcast::Sender<ActorMessage>;

    /// gets the message receiver
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

    /// run
    fn run(mut self) {
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
                            message: Message::Action(Action::Exit),
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

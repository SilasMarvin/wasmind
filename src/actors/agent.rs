use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing::info;
use uuid::Uuid;

use crate::{
    actors::{
        assistant::Assistant,
        tools::{
            command::Command, edit_file::EditFile, file_reader::FileReaderActor, mcp::MCP,
            plan_approval::PlanApproval, planner::Planner, spawn_agent::SpawnAgent,
        },
        Actor, Message,
    },
    config::ParsedConfig,
};

/// Unique identifier for an agent
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    pub fn new() -> Self {
        AgentId(Uuid::new_v4().to_string())
    }
}

/// Unique identifier for a task
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new() -> Self {
        TaskId(Uuid::new_v4().to_string())
    }
}

/// Agent behavior types
#[derive(Debug, Clone)]
pub enum AgentBehavior {
    Manager(ManagerLogic),
    Worker(WorkerLogic),
}

/// Manager agent logic
#[derive(Debug, Clone)]
pub struct ManagerLogic {
    pub id: AgentId,
    pub role: String,
}

/// Worker agent logic
#[derive(Debug, Clone)]
pub struct WorkerLogic {
    pub id: AgentId,
    pub role: String,
}

/// Task awaiting manager decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskAwaitingManager {
    AwaitingPlanApproval(crate::actors::tools::planner::TaskPlan),
    AwaitingMoreInformation(String),
}

/// Task status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskStatus {
    Done(Result<String, String>),
    InProgress,
    AwaitingManager(TaskAwaitingManager),
}

/// Inter-agent message for communication between agents
#[derive(Debug, Clone)]
pub enum InterAgentMessage {
    /// Agent reports task status to manager
    TaskStatusUpdate {
        task_id: TaskId,
        status: TaskStatus,
        from_agent: AgentId,
    },
    /// Manager approves a plan
    PlanApproved {
        task_id: TaskId,
        plan_id: String,
    },
    /// Manager rejects a plan
    PlanRejected {
        task_id: TaskId,
        plan_id: String,
        reason: String,
    },
}

/// Response when spawning a new agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpawnedResponse {
    pub agent_id: AgentId,
    pub task_id: TaskId,
    pub agent_role: String,
}

/// Agent instance that can be either a Manager or Worker
pub struct Agent {
    pub behavior: AgentBehavior,
    pub config: ParsedConfig,
    pub task_id: TaskId,
    pub task_description: String,
    /// Channel to send messages back to the parent manager
    pub parent_tx: Option<broadcast::Sender<InterAgentMessage>>,
    /// Channel to receive messages from child agents (for managers)
    pub child_rx: Option<broadcast::Receiver<InterAgentMessage>>,
    /// Channel to send messages to child agents (for managers)
    pub child_tx: Option<broadcast::Sender<InterAgentMessage>>,
    /// Internal message channel for this agent's actors
    internal_tx: Option<broadcast::Sender<Message>>,
}

impl Agent {
    /// Create a new Manager agent with a task
    pub fn new_manager(role: String, task_description: String, mut config: ParsedConfig) -> Self {
        let id = AgentId::new();
        let task_id = TaskId::new();
        let behavior = AgentBehavior::Manager(ManagerLogic { id, role: role.clone() });
        
        // Managers need child communication channels
        let (child_tx, child_rx) = broadcast::channel(1024);
        
        // Use the appropriate model config based on role
        if role == "Main Manager" {
            if let Some(main_manager_model) = &config.hive.main_manager_model {
                config.model = main_manager_model.clone();
            }
        } else {
            if let Some(sub_manager_model) = &config.hive.sub_manager_model {
                config.model = sub_manager_model.clone();
            }
        }
        
        Agent {
            behavior,
            config,
            task_id,
            task_description,
            parent_tx: None,
            child_rx: Some(child_rx),
            child_tx: Some(child_tx),
            internal_tx: None,
        }
    }

    /// Create a new Worker agent with a task
    pub fn new_worker(role: String, task_description: String, mut config: ParsedConfig) -> Self {
        let id = AgentId::new();
        let task_id = TaskId::new();
        let behavior = AgentBehavior::Worker(WorkerLogic { id, role });
        
        // Use the worker model config if available
        if let Some(worker_model) = &config.hive.worker_model {
            config.model = worker_model.clone();
        }
        
        Agent {
            behavior,
            config,
            task_id,
            task_description,
            parent_tx: None,
            child_rx: None,
            child_tx: None,
            internal_tx: None,
        }
    }

    /// Get the agent's ID
    pub fn id(&self) -> &AgentId {
        match &self.behavior {
            AgentBehavior::Manager(logic) => &logic.id,
            AgentBehavior::Worker(logic) => &logic.id,
        }
    }

    /// Get the agent's role
    pub fn role(&self) -> &str {
        match &self.behavior {
            AgentBehavior::Manager(logic) => &logic.role,
            AgentBehavior::Worker(logic) => &logic.role,
        }
    }
    
    /// Get the internal message sender
    pub fn get_internal_tx(&self) -> Option<&broadcast::Sender<Message>> {
        self.internal_tx.as_ref()
    }

    /// Get the required actors for this agent type
    pub fn get_required_actors(&self) -> Vec<&'static str> {
        match &self.behavior {
            AgentBehavior::Manager(_) => {
                vec![
                    Assistant::ACTOR_ID,
                    Planner::ACTOR_ID,
                    SpawnAgent::ACTOR_ID,
                    PlanApproval::ACTOR_ID,
                ]
            }
            AgentBehavior::Worker(_) => {
                vec![
                    Assistant::ACTOR_ID,
                    Command::ACTOR_ID,
                    FileReaderActor::ACTOR_ID,
                    EditFile::ACTOR_ID,
                    Planner::ACTOR_ID,
                    MCP::ACTOR_ID,
                ]
            }
        }
    }


    /// Start the agent's actors based on its behavior type
    pub async fn start_actors(&self) -> broadcast::Sender<Message> {
        info!("Starting actors for agent {} ({})", self.id().0, self.role());

        // Create broadcast channel for internal actor communication
        let (tx, _) = broadcast::channel::<Message>(1024);

        // Create shared file reader
        let file_reader = Arc::new(Mutex::new(
            crate::actors::tools::file_reader::FileReader::new(),
        ));

        match &self.behavior {
            AgentBehavior::Manager(_) => {
                // Managers only get planning and agent management tools
                Assistant::new(self.config.clone(), tx.clone()).run();
                Planner::new(self.config.clone(), tx.clone()).run();
                
                // Add spawn_agent and plan approval tools for managers
                if let Some(child_tx) = &self.child_tx {
                    SpawnAgent::new_with_channel(
                        self.config.clone(),
                        tx.clone(),
                        child_tx.clone(),
                    ).run();
                    
                    PlanApproval::new_with_channel(
                        self.config.clone(),
                        tx.clone(),
                        child_tx.clone(),
                    ).run();
                }
            }
            AgentBehavior::Worker(_) => {
                // Workers get all execution tools
                Assistant::new(self.config.clone(), tx.clone()).run();
                Command::new(self.config.clone(), tx.clone()).run();
                FileReaderActor::with_file_reader(
                    self.config.clone(),
                    tx.clone(),
                    file_reader.clone(),
                )
                .run();
                EditFile::with_file_reader(self.config.clone(), tx.clone(), file_reader).run();
                Planner::new(self.config.clone(), tx.clone()).run();
                MCP::new(self.config.clone(), tx.clone()).run();
            }
        }

        tx
    }

    /// Run the agent's main loop
    pub async fn run(mut self) {
        let message_tx = self.start_actors().await;
        self.internal_tx = Some(message_tx.clone());
        let mut message_rx = message_tx.subscribe();

        // Track which actors are ready
        let mut ready_actors = std::collections::HashSet::new();
        let required_actors = self.get_required_actors();
        let mut initial_prompt_sent = false;

        // Main agent loop
        let mut task_completed = false;
        loop {
            tokio::select! {
                // Handle internal messages from actors
                Ok(msg) = message_rx.recv() => {
                    match &msg {
                        Message::ActorReady { actor_id } => {
                            info!("Agent {}: Actor {} is ready", self.id().0, actor_id);
                            ready_actors.insert(*actor_id);

                            // Check if all required actors are ready and we haven't sent the initial prompt yet
                            if !initial_prompt_sent
                                && required_actors.iter().all(|id| ready_actors.contains(id))
                            {
                                info!("Agent {}: All actors ready, starting task", self.id().0);
                                self.send_initial_prompt(&message_tx).await;
                                initial_prompt_sent = true;
                            }
                        }
                        Message::AssistantResponse(content) => {
                            // Check if assistant has finished (no tool calls)
                            match content {
                                genai::chat::MessageContent::Text(_) |
                                genai::chat::MessageContent::Parts(_) => {
                                    info!("Agent {}: Task completed", self.id().0);
                                    task_completed = true;
                                }
                                _ => {}
                            }
                            self.handle_internal_message(msg).await;
                        }
                        _ => {
                            self.handle_internal_message(msg).await;
                        }
                    }
                    
                    if task_completed {
                        break;
                    }
                }
                // Handle messages from child agents (for managers)
                Some(Ok(msg)) = async {
                    if let Some(rx) = &mut self.child_rx {
                        Some(rx.recv().await)
                    } else {
                        None
                    }
                } => {
                    self.handle_child_message(msg).await;
                }
            }
        }

        // Report completion to parent if we have one
        if let Some(parent_tx) = &self.parent_tx {
            let _ = parent_tx.send(InterAgentMessage::TaskStatusUpdate {
                task_id: self.task_id.clone(),
                status: TaskStatus::Done(Ok("Task completed".to_string())),
                from_agent: self.id().clone(),
            });
        }
    }

    /// Send the initial prompt to the assistant based on agent type
    async fn send_initial_prompt(&self, message_tx: &broadcast::Sender<Message>) {
        // Send initial message to start the task
        let _ = message_tx.send(Message::UserTUIInput(
            "Please analyze your objective and determine the best approach to accomplish it.".to_string()
        ));
    }

    /// Handle internal messages from the agent's actors
    async fn handle_internal_message(&mut self, message: Message) {
        // Handle messages that apply to both managers and workers
        match &message {
            Message::PlanUpdated(plan) => {
                // If we're a worker and have a parent, report the plan for approval
                if let AgentBehavior::Worker(_) = &self.behavior {
                    if let Some(parent_tx) = &self.parent_tx {
                        let _ = parent_tx.send(InterAgentMessage::TaskStatusUpdate {
                            task_id: self.task_id.clone(),
                            status: TaskStatus::AwaitingManager(
                                TaskAwaitingManager::AwaitingPlanApproval(plan.clone())
                            ),
                            from_agent: self.id().clone(),
                        });
                    }
                }
            }
            Message::ToolCallUpdate(update) => {
                // If a tool finishes with an error, we might want to report it
                if let crate::actors::ToolCallStatus::Finished(Err(error)) = &update.status {
                    info!("Agent {}: Tool call {} failed: {}", self.id().0, update.call_id, error);
                }
            }
            _ => {}
        }
    }

    /// Handle messages from child agents (only for managers)
    async fn handle_child_message(&mut self, message: InterAgentMessage) {
        if let AgentBehavior::Manager(_) = &self.behavior {
            match message {
                InterAgentMessage::TaskStatusUpdate { task_id, status, from_agent } => {
                    info!("Manager received status update for task {} from agent {}", task_id.0, from_agent.0);
                    
                    // Format the status update as a message to the manager's LLM
                    let status_message = match &status {
                        TaskStatus::Done(Ok(result)) => {
                            format!(
                                "Agent {} has completed task {}.\nResult: {}",
                                from_agent.0, task_id.0, result
                            )
                        }
                        TaskStatus::Done(Err(error)) => {
                            format!(
                                "Agent {} failed to complete task {}.\nError: {}",
                                from_agent.0, task_id.0, error
                            )
                        }
                        TaskStatus::InProgress => {
                            format!(
                                "Agent {} is working on task {}.",
                                from_agent.0, task_id.0
                            )
                        }
                        TaskStatus::AwaitingManager(TaskAwaitingManager::AwaitingPlanApproval(plan)) => {
                            format!(
                                "Agent {} has submitted a plan for task {} and is awaiting your approval.\n\nPlan:\n{}",
                                from_agent.0, task_id.0, plan
                            )
                        }
                        TaskStatus::AwaitingManager(TaskAwaitingManager::AwaitingMoreInformation(info)) => {
                            format!(
                                "Agent {} needs more information for task {}:\n{}",
                                from_agent.0, task_id.0, info
                            )
                        }
                    };
                    
                    // Find the internal message_tx to send this update to the manager's assistant
                    if let Some(tx) = self.get_internal_tx() {
                        let _ = tx.send(Message::UserTUIInput(status_message));
                    }
                }
                InterAgentMessage::PlanApproved { task_id, plan_id } => {
                    info!("Agent received plan approval for task {} plan {}", task_id.0, plan_id);
                    // Workers will handle this in their planner actor
                }
                InterAgentMessage::PlanRejected { task_id, plan_id, reason } => {
                    info!("Agent received plan rejection for task {} plan {}: {}", task_id.0, plan_id, reason);
                    // Workers will handle this in their planner actor
                }
            }
        }
    }
}
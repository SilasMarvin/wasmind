use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tracing::info;
use uuid::Uuid;

use crate::{
    actors::{
        Actor, Message,
        assistant::Assistant,
        state_system::StateSystem,
        tools::{
            command::Command, complete::Complete, edit_file::EditFile,
            file_reader::FileReaderActor, mcp::MCP, plan_approval::PlanApproval, planner::Planner,
            spawn_agent::SpawnAgent,
        },
    },
    config::ParsedConfig,
};

/// Role name for the main manager agent
pub const MAIN_MANAGER_ROLE: &str = "Main Manager";

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
    PlanApproved { task_id: TaskId, plan_id: String },
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
    /// Current state of the agent
    state: AgentState,
}

impl Agent {
    /// Create a new Manager agent with a task
    pub fn new_manager(role: String, task_description: String, mut config: ParsedConfig) -> Self {
        let id = AgentId::new();
        let task_id = TaskId::new();
        let behavior = AgentBehavior::Manager(ManagerLogic {
            id,
            role: role.clone(),
        });

        // Managers need child communication channels
        let (child_tx, child_rx) = broadcast::channel(1024);

        // Use the appropriate model config based on role
        if role == MAIN_MANAGER_ROLE {
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
            state: AgentState::Initializing,
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
            state: AgentState::Initializing,
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
                let mut actors = vec![
                    Assistant::ACTOR_ID,
                    Planner::ACTOR_ID,
                    SpawnAgent::ACTOR_ID,
                    PlanApproval::ACTOR_ID,
                ];

                // Add complete tool for sub-managers or Main Manager in headless mode
                if self.role() != MAIN_MANAGER_ROLE || cfg!(not(feature = "gui")) {
                    actors.push(Complete::ACTOR_ID);
                }

                actors
            }
            AgentBehavior::Worker(_) => {
                vec![
                    Assistant::ACTOR_ID,
                    Command::ACTOR_ID,
                    FileReaderActor::ACTOR_ID,
                    EditFile::ACTOR_ID,
                    Planner::ACTOR_ID,
                    MCP::ACTOR_ID,
                    Complete::ACTOR_ID,
                ]
            }
        }
    }

    /// Start the agent's actors based on its behavior type
    #[tracing::instrument(name = "start_actors", skip(self), fields(agent_id = %self.id().0, role = %self.role()))]
    pub async fn start_actors(&self) -> broadcast::Sender<Message> {
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

                // Add complete tool for sub-managers or Main Manager in headless mode
                if self.role() != MAIN_MANAGER_ROLE || cfg!(not(feature = "gui")) {
                    Complete::new(self.config.clone(), tx.clone()).run();
                }

                // Add spawn_agent and plan approval tools for managers
                if let Some(child_tx) = &self.child_tx {
                    SpawnAgent::new_with_channel(self.config.clone(), tx.clone(), child_tx.clone())
                        .run();

                    PlanApproval::new_with_channel(
                        self.config.clone(),
                        tx.clone(),
                        child_tx.clone(),
                    )
                    .run();
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
                Complete::new(self.config.clone(), tx.clone()).run();
            }
        }

        tx
    }

    /// Run the agent's main loop
    #[tracing::instrument(name = "agent_run", skip(self), fields(agent_id = %self.id().0, role = %self.role(), agent_type = ?self.behavior))]
    pub async fn run(mut self) {
        let message_tx = self.start_actors().await;
        self.internal_tx = Some(message_tx.clone());
        let mut message_rx = message_tx.subscribe();

        // Track which actors are ready
        let mut ready_actors = std::collections::HashSet::new();
        let required_actors = self.get_required_actors();
        let mut initial_prompt_sent = false;

        // Transition to WaitingForActors state
        self.state = AgentState::WaitingForActors {
            ready_actors: ready_actors.clone(),
            required_actors: required_actors.clone(),
        };

        // Main agent loop
        let mut task_completed = false;
        loop {
            tokio::select! {
                // Handle internal messages from actors
                Ok(msg) = message_rx.recv() => {
                    tracing::debug!(name = "agent_received_internal_message", message = ?msg);

                    // First, attempt state transition
                    self.transition(&msg);

                    match &msg {
                        Message::ActorReady { actor_id } => {
                            ready_actors.insert(*actor_id);

                            // Check if all required actors are ready and we haven't sent the initial prompt yet
                            if !initial_prompt_sent
                                && required_actors.iter().all(|id| ready_actors.contains(id))
                            {
                                self.send_initial_prompt(&message_tx).await;
                                initial_prompt_sent = true;
                            }
                        }
                        Message::TaskCompleted { summary: _, success: _ } => {
                            // Agent explicitly signaled task completion via Complete tool
                            task_completed = true;
                            self.handle_internal_message(msg).await;
                        }
                        Message::AssistantResponse(content) => {
                            // Check if assistant has finished (no tool calls) - treat as error
                            match content {
                                genai::chat::MessageContent::Text(text) => {
                                    // Report failure with the text content
                                    if let Some(parent_tx) = &self.parent_tx {
                                        let _ = parent_tx.send(InterAgentMessage::TaskStatusUpdate {
                                            task_id: self.task_id.clone(),
                                            status: TaskStatus::Done(Err(format!("Agent failed to use complete tool properly. Last response: {}", text))),
                                            from_agent: self.id().clone(),
                                        });
                                    }
                                    task_completed = true;
                                }
                                genai::chat::MessageContent::Parts(parts) => {
                                    // Extract text from parts and report failure
                                    let text_content = parts.iter()
                                        .filter_map(|part| match part {
                                            genai::chat::ContentPart::Text(text) => Some(text.as_str()),
                                            _ => None,
                                        })
                                        .collect::<Vec<_>>()
                                        .join(" ");

                                    if let Some(parent_tx) = &self.parent_tx {
                                        let _ = parent_tx.send(InterAgentMessage::TaskStatusUpdate {
                                            task_id: self.task_id.clone(),
                                            status: TaskStatus::Done(Err(format!("Agent failed to use complete tool properly. Last response: {}", text_content))),
                                            from_agent: self.id().clone(),
                                        });
                                    }
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
                    tracing::debug!(name = "agent_received_child_message", message = ?msg);
                    self.handle_child_message(msg).await;
                }
            }
        }

        // Agent finished - this should have been handled by TaskCompleted message
        // If we reach here without explicit completion, it's likely an error
        if let Some(parent_tx) = &self.parent_tx {
            let _ = parent_tx.send(InterAgentMessage::TaskStatusUpdate {
                task_id: self.task_id.clone(),
                status: TaskStatus::Done(Err(
                    "Agent exited without explicit completion".to_string()
                )),
                from_agent: self.id().clone(),
            });
        }
    }

    /// Send the initial prompt to the assistant based on agent type
    #[tracing::instrument(name = "send_initial_prompt", skip(self, message_tx), fields(agent_id = %self.id().0))]
    async fn send_initial_prompt(&self, message_tx: &broadcast::Sender<Message>) {
        // Send initial message to start the task
        let _ = message_tx.send(Message::UserTUIInput(
            "Please analyze your objective and determine the best approach to accomplish it."
                .to_string(),
        ));
    }

    /// Handle internal messages from the agent's actors
    #[tracing::instrument(name = "handle_internal_message", skip(self, message), fields(agent_id = %self.id().0, message_type = ?std::mem::discriminant(&message)))]
    async fn handle_internal_message(&mut self, message: Message) {
        // Handle messages that apply to both managers and workers
        match &message {
            Message::TaskCompleted { summary, success } => {
                // Report task completion to parent if we have one
                if let Some(parent_tx) = &self.parent_tx {
                    let status = if *success {
                        TaskStatus::Done(Ok(summary.clone()))
                    } else {
                        TaskStatus::Done(Err(summary.clone()))
                    };

                    let _ = parent_tx.send(InterAgentMessage::TaskStatusUpdate {
                        task_id: self.task_id.clone(),
                        status,
                        from_agent: self.id().clone(),
                    });
                }
            }
            Message::PlanUpdated(plan) => {
                // If we're a worker and have a parent, report the plan for approval
                if let AgentBehavior::Worker(_) = &self.behavior {
                    if let Some(parent_tx) = &self.parent_tx {
                        let _ = parent_tx.send(InterAgentMessage::TaskStatusUpdate {
                            task_id: self.task_id.clone(),
                            status: TaskStatus::AwaitingManager(
                                TaskAwaitingManager::AwaitingPlanApproval(plan.clone()),
                            ),
                            from_agent: self.id().clone(),
                        });
                    }
                }
            }
            Message::ToolCallUpdate(update) => {
                // If a tool finishes with an error, we might want to report it
                if let crate::actors::ToolCallStatus::Finished(Err(error)) = &update.status {
                    info!(
                        "Agent {}: Tool call {} failed: {}",
                        self.id().0,
                        update.call_id,
                        error
                    );
                }
            }
            _ => {}
        }
    }

    /// Handle messages from child agents (only for managers)
    async fn handle_child_message(&mut self, message: InterAgentMessage) {
        if let AgentBehavior::Manager(_) = &self.behavior {
            match message {
                InterAgentMessage::TaskStatusUpdate {
                    task_id,
                    status,
                    from_agent,
                } => {
                    // Send AgentStatusUpdate to update system state
                    if let Some(tx) = self.get_internal_tx() {
                        let _ = tx.send(Message::AgentStatusUpdate {
                            agent_id: from_agent.clone(),
                            status: status.clone(),
                        });

                        // If task is done, remove the agent from tracking
                        if matches!(&status, TaskStatus::Done(_)) {
                            let _ = tx.send(Message::AgentRemoved {
                                agent_id: from_agent.clone(),
                            });
                        }
                    }

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
                            format!("Agent {} is working on task {}.", from_agent.0, task_id.0)
                        }
                        TaskStatus::AwaitingManager(TaskAwaitingManager::AwaitingPlanApproval(
                            plan,
                        )) => {
                            format!(
                                "Agent {} has submitted a plan for task {} and is awaiting your approval.\n\nPlan:\n{}",
                                from_agent.0, task_id.0, plan
                            )
                        }
                        TaskStatus::AwaitingManager(
                            TaskAwaitingManager::AwaitingMoreInformation(info),
                        ) => {
                            format!(
                                "Agent {} needs more information for task {}:\n{}",
                                from_agent.0, task_id.0, info
                            )
                        }
                    };

                    if let Some(tx) = self.get_internal_tx() {
                        let _ = tx.send(Message::UserTUIInput(status_message));
                    }
                }
                InterAgentMessage::PlanApproved { task_id: _, plan_id: _ } => {
                    // Workers will handle this in their planner actor
                }
                InterAgentMessage::PlanRejected {
                    task_id: _,
                    plan_id: _,
                    reason: _,
                } => {
                    // Workers will handle this in their planner actor
                }
            }
        }
    }
}

/// States that an Agent can be in during its lifecycle
#[derive(Debug, Clone, PartialEq)]
pub enum AgentState {
    /// Agent is initializing and starting actors
    Initializing,
    /// Waiting for all required actors to be ready
    WaitingForActors {
        ready_actors: std::collections::HashSet<&'static str>,
        required_actors: Vec<&'static str>,
    },
    /// Agent is active and processing its task
    Active,
    /// Worker agent waiting for manager approval of plan
    WaitingForApproval { plan_id: String },
    /// Manager agent waiting for spawned sub-agents to complete
    WaitingForSubAgents { agent_ids: Vec<AgentId> },
    /// Agent is completing its task and preparing final report
    Completing,
    /// Agent has terminated (successfully or with error)
    Terminated { result: Result<String, String> },
}

impl crate::actors::state_system::StateSystem for Agent {
    type State = AgentState;

    fn current_state(&self) -> &Self::State {
        &self.state
    }

    fn transition(&mut self, message: &Message) -> Option<Self::State> {
        use AgentState::*;

        let new_state = match (&self.state, message) {
            // Actor ready messages during initialization
            (
                WaitingForActors {
                    ready_actors,
                    required_actors,
                },
                Message::ActorReady { actor_id },
            ) => {
                let mut new_ready = ready_actors.clone();
                new_ready.insert(actor_id);

                if required_actors.iter().all(|id| new_ready.contains(id)) {
                    Some(Active)
                } else {
                    Some(WaitingForActors {
                        ready_actors: new_ready,
                        required_actors: required_actors.clone(),
                    })
                }
            }

            // Plan created by worker - waiting for approval
            (Active, Message::PlanUpdated(_))
                if matches!(&self.behavior, AgentBehavior::Worker(_)) =>
            {
                Some(WaitingForApproval {
                    plan_id: format!("plan_{}", self.task_id.0),
                })
            }

            // Plan approved/rejected - back to active
            (WaitingForApproval { .. }, Message::AssistantToolCall(tool_call))
                if tool_call.fn_name == "approve_plan" || tool_call.fn_name == "reject_plan" =>
            {
                Some(Active)
            }

            // Manager spawned agents - track them
            (Active, Message::AgentSpawned { agent_id, .. })
                if matches!(&self.behavior, AgentBehavior::Manager(_)) =>
            {
                Some(WaitingForSubAgents {
                    agent_ids: vec![agent_id.clone()],
                })
            }

            // More agents spawned while already waiting
            (WaitingForSubAgents { agent_ids }, Message::AgentSpawned { agent_id, .. }) => {
                let mut new_ids = agent_ids.clone();
                new_ids.push(agent_id.clone());
                Some(WaitingForSubAgents { agent_ids: new_ids })
            }

            // Sub-agent completed
            (
                WaitingForSubAgents { agent_ids },
                Message::AgentStatusUpdate { agent_id, status },
            ) if matches!(status, TaskStatus::Done(_)) => {
                let mut remaining = agent_ids.clone();
                remaining.retain(|id| id != agent_id);

                if remaining.is_empty() {
                    Some(Active)
                } else {
                    Some(WaitingForSubAgents {
                        agent_ids: remaining,
                    })
                }
            }

            // Task completion
            (Active, Message::AssistantResponse(_)) => {
                // In a real implementation, we'd check if this response indicates completion
                None
            }

            _ => None,
        };

        if let Some(ref new_state) = new_state {
            tracing::info!(
                "Agent {} state transition: {:?} -> {:?}",
                self.id().0,
                self.state,
                new_state
            );
            self.state = new_state.clone();
        }

        new_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::state_system::StateSystem;
    use crate::actors::state_system::test_utils::*;
    use crate::config::{Config, ParsedConfig};

    fn create_test_agent(is_manager: bool) -> Agent {
        let config: ParsedConfig = Config::default().unwrap().try_into().unwrap();
        if is_manager {
            Agent::new_manager("Test Manager".to_string(), "Test task".to_string(), config)
        } else {
            Agent::new_worker("Test Worker".to_string(), "Test task".to_string(), config)
        }
    }

    #[test]
    fn test_agent_creation() {
        let worker = create_test_agent(false);
        assert_eq!(worker.role(), "Test Worker");
        assert!(matches!(worker.behavior, AgentBehavior::Worker(_)));
        assert_eq!(worker.state, AgentState::Initializing);

        let manager = create_test_agent(true);
        assert_eq!(manager.role(), "Test Manager");
        assert!(matches!(manager.behavior, AgentBehavior::Manager(_)));
        assert_eq!(manager.state, AgentState::Initializing);
    }

    #[test]
    fn test_agent_required_actors() {
        let worker = create_test_agent(false);
        let worker_actors = worker.get_required_actors();
        assert!(worker_actors.contains(&"assistant"));
        assert!(worker_actors.contains(&"command"));
        assert!(!worker_actors.contains(&"spawn_agent"));

        let manager = create_test_agent(true);
        let manager_actors = manager.get_required_actors();
        assert!(manager_actors.contains(&"assistant"));
        assert!(manager_actors.contains(&"spawn_agent"));
        assert!(!manager_actors.contains(&"command"));
    }

    #[test]
    fn test_agent_state_waiting_for_actors() {
        let mut agent = create_test_agent(false);
        let required = agent.get_required_actors();

        // Start in WaitingForActors state
        agent.state = AgentState::WaitingForActors {
            ready_actors: std::collections::HashSet::new(),
            required_actors: required.clone(),
        };

        // Add actors one by one
        for (i, actor_id) in required.iter().enumerate() {
            let is_last = i == required.len() - 1;

            if is_last {
                // Last actor should transition to Active
                assert_state_transition(
                    &mut agent,
                    Message::ActorReady { actor_id },
                    AgentState::Active,
                );
            } else {
                // Not all actors ready yet
                agent.transition(&Message::ActorReady { actor_id });
                assert!(matches!(agent.state, AgentState::WaitingForActors { .. }));
            }
        }
    }

    #[test]
    fn test_worker_plan_approval_flow() {
        let mut worker = create_test_agent(false);
        worker.state = AgentState::Active;

        // Worker creates a plan
        let plan = crate::actors::tools::planner::TaskPlan {
            title: "Test Plan".to_string(),
            tasks: vec![],
        };

        let expected_plan_id = format!("plan_{}", worker.task_id.0);
        assert_state_transition(
            &mut worker,
            Message::PlanUpdated(plan),
            AgentState::WaitingForApproval {
                plan_id: expected_plan_id,
            },
        );
    }

    #[test]
    fn test_manager_spawning_agents() {
        let mut manager = create_test_agent(true);
        manager.state = AgentState::Active;

        let agent_id = AgentId("sub-agent-1".to_string());

        assert_state_transition(
            &mut manager,
            Message::AgentSpawned {
                agent_id: agent_id.clone(),
                agent_role: "Worker".to_string(),
                task_id: TaskId("task-1".to_string()),
                task_description: "Do work".to_string(),
            },
            AgentState::WaitingForSubAgents {
                agent_ids: vec![agent_id],
            },
        );
    }

    #[test]
    fn test_manager_multiple_sub_agents() {
        let mut manager = create_test_agent(true);
        let agent_id1 = AgentId("sub-1".to_string());
        let agent_id2 = AgentId("sub-2".to_string());

        // Start with one sub-agent
        manager.state = AgentState::WaitingForSubAgents {
            agent_ids: vec![agent_id1.clone()],
        };

        // Spawn another
        assert_state_transition(
            &mut manager,
            Message::AgentSpawned {
                agent_id: agent_id2.clone(),
                agent_role: "Worker".to_string(),
                task_id: TaskId("task-2".to_string()),
                task_description: "More work".to_string(),
            },
            AgentState::WaitingForSubAgents {
                agent_ids: vec![agent_id1.clone(), agent_id2.clone()],
            },
        );

        // First agent completes
        assert_state_transition(
            &mut manager,
            Message::AgentStatusUpdate {
                agent_id: agent_id1,
                status: TaskStatus::Done(Ok("Success".to_string())),
            },
            AgentState::WaitingForSubAgents {
                agent_ids: vec![agent_id2.clone()],
            },
        );

        // Second agent completes - back to Active
        assert_state_transition(
            &mut manager,
            Message::AgentStatusUpdate {
                agent_id: agent_id2,
                status: TaskStatus::Done(Ok("Also success".to_string())),
            },
            AgentState::Active,
        );
    }

    #[test]
    fn test_agent_communication_channels() {
        let config: ParsedConfig = Config::default().unwrap().try_into().unwrap();
        let (parent_tx, _) = broadcast::channel(100);
        let (child_tx, _) = broadcast::channel(100);

        let mut worker =
            Agent::new_worker("Worker".to_string(), "Task".to_string(), config.clone());
        worker.parent_tx = Some(parent_tx.clone());
        assert!(worker.parent_tx.is_some());

        let mut manager = Agent::new_manager("Manager".to_string(), "Task".to_string(), config);
        manager.child_tx = Some(child_tx.clone());
        assert!(manager.child_tx.is_some());
    }
}

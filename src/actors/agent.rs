use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast::Sender};
use uuid::Uuid;

use crate::{
    actors::{
        Actor,
        assistant::Assistant,
        tools::{
            command::Command, complete::Complete, edit_file::EditFile,
            file_reader::FileReaderActor, mcp::MCP, plan_approval::PlanApproval, planner::Planner,
            spawn_agent::SpawnAgent,
        },
    },
    config::ParsedConfig,
};

use super::{ActorMessage, tools::file_reader::FileReader};

/// Role name for the main manager agent
pub const MAIN_MANAGER_ROLE: &str = "Main Manager";

/// Agent behavior types
#[derive(Debug, Clone)]
pub enum AgentBehavior {
    MainManager(ManagerLogic),
    Manager(ManagerLogic),
    Worker(WorkerLogic),
}

/// Manager agent logic
#[derive(Debug, Clone)]
pub struct ManagerLogic {
    pub id: Uuid,
    pub role: String,
}

/// Worker agent logic
#[derive(Debug, Clone)]
pub struct WorkerLogic {
    pub id: Uuid,
    pub role: String,
}

/// Response when spawning a new agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpawnedResponse {
    pub agent_id: Uuid,
    pub agent_role: String,
}

/// Agent instance that can be either a Manager or Worker
pub struct Agent {
    tx: Sender<ActorMessage>,
    pub behavior: AgentBehavior,
    pub config: ParsedConfig,
    pub task_description: Option<String>,
    /// Parent scope
    parent_scope: Uuid,
    /// Agent scope
    scope: Uuid,
}

impl Agent {
    /// Create a new Manager agent with a task
    pub fn new_manager(
        tx: Sender<ActorMessage>,
        role: String,
        task_description: Option<String>,
        config: ParsedConfig,
        parent_scope: Uuid,
    ) -> Self {
        let id = Uuid::new_v4();

        let behavior = AgentBehavior::Manager(ManagerLogic {
            id: id.clone(),
            role: role.clone(),
        });

        Agent {
            tx,
            behavior,
            config,
            task_description,
            parent_scope,
            scope: id,
        }
    }

    /// Create a new Worker agent with a task
    pub fn new_worker(
        tx: Sender<ActorMessage>,
        role: String,
        task_description: Option<String>,
        config: ParsedConfig,
        parent_scope: Uuid,
    ) -> Self {
        let id = Uuid::new_v4();
        let behavior = AgentBehavior::Worker(WorkerLogic {
            id: id.clone(),
            role,
        });

        Agent {
            tx,
            behavior,
            config,
            task_description,
            parent_scope,
            scope: id,
        }
    }

    /// Get the agent's ID
    pub fn id(&self) -> &Uuid {
        match &self.behavior {
            AgentBehavior::MainManager(logic) => &logic.id,
            AgentBehavior::Manager(logic) => &logic.id,
            AgentBehavior::Worker(logic) => &logic.id,
        }
    }

    /// Get the agent's role
    pub fn role(&self) -> &str {
        match &self.behavior {
            AgentBehavior::MainManager(logic) => &logic.role,
            AgentBehavior::Manager(logic) => &logic.role,
            AgentBehavior::Worker(logic) => &logic.role,
        }
    }

    /// Get the required actors for this agent's assistant type
    pub fn get_required_actors(&self) -> Vec<&'static str> {
        match &self.behavior {
            AgentBehavior::Manager(_) | AgentBehavior::MainManager(_) => {
                let mut actors = vec![
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

    /// Run the agent - start their actors
    #[tracing::instrument(name = "agent_run", skip(self), fields(agent_id = %self.id(), role = %self.role(), agent_type = ?self.behavior))]
    pub async fn run(self) {
        // Create shared file reader
        let file_reader = Arc::new(Mutex::new(FileReader::default()));

        match &self.behavior {
            AgentBehavior::Manager(_) | AgentBehavior::MainManager(_) => {
                let config = if matches!(self.behavior, AgentBehavior::MainManager(_)) {
                    self.config.hive.main_manager_model.clone()
                } else {
                    self.config.hive.sub_manager_model.clone()
                };

                // Managers only get planning and agent management tools
                Assistant::new(
                    config,
                    self.tx.clone(),
                    self.scope.clone(),
                    self.get_required_actors(),
                    self.task_description.clone(),
                )
                .run();
                Planner::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();

                // Add complete tool for sub-managers or Main Manager in headless mode
                if self.role() != MAIN_MANAGER_ROLE || cfg!(not(feature = "gui")) {
                    Complete::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
                }

                // Add spawn_agent and plan approval tools for managers
                SpawnAgent::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();

                PlanApproval::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
            }
            AgentBehavior::Worker(_) => {
                // Workers get all execution tools
                Assistant::new(
                    self.config.hive.worker_model.clone(),
                    self.tx.clone(),
                    self.scope.clone(),
                    self.get_required_actors(),
                    self.task_description.clone(),
                )
                .run();
                Command::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
                FileReaderActor::new(
                    self.config.clone(),
                    self.tx.clone(),
                    file_reader.clone(),
                    self.scope.clone(),
                )
                .run();
                EditFile::new(
                    self.config.clone(),
                    self.tx.clone(),
                    file_reader,
                    self.scope.clone(),
                )
                .run();
                Planner::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
                MCP::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
                Complete::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
            }
        }
    }
}

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast::Sender};

use crate::{
    actors::{
        Actor,
        assistant::Assistant,
        tools::{
            command::Command, complete::Complete, edit_file::EditFile,
            file_reader::FileReaderActor, mcp::MCP, planner::Planner,
            send_manager_message::SendManagerMessage, send_message::SendMessage,
            spawn_agent::SpawnAgent,
        },
    },
    config::ParsedConfig,
    scope::Scope,
};

use super::{
    ActorMessage, AgentType,
    tools::{file_reader::FileReader, wait::Wait},
};

/// Role name for the main manager agent
pub const MAIN_MANAGER_ROLE: &str = "Main Manager";

/// Response when spawning a new agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpawnedResponse {
    pub agent_id: Scope,
    pub agent_role: String,
}

/// Agent instance that can be either a Manager or Worker
pub struct Agent {
    tx: Sender<ActorMessage>,
    pub r#type: AgentType,
    pub config: ParsedConfig,
    pub task_description: Option<String>,
    pub role: String,
    /// Parent scope
    pub parent_scope: Scope,
    /// Agent scope
    pub scope: Scope,
}

impl Agent {
    pub fn new(
        tx: Sender<ActorMessage>,
        role: String,
        task_description: Option<String>,
        config: ParsedConfig,
        parent_scope: Scope,
        r#type: AgentType,
    ) -> Self {
        let id = Scope::new();

        Self {
            tx,
            r#type,
            config,
            task_description,
            parent_scope,
            scope: id,
            role,
        }
    }

    pub fn new_with_scope(
        tx: Sender<ActorMessage>,
        role: String,
        task_description: Option<String>,
        config: ParsedConfig,
        scope: Scope,
        parent_scope: Scope,
        r#type: AgentType,
    ) -> Self {
        Self {
            tx,
            r#type,
            config,
            task_description,
            scope,
            parent_scope,
            role,
        }
    }

    /// Get the required actors for this agent's assistant type
    pub fn get_required_actors(&self) -> Vec<&'static str> {
        match &self.r#type {
            AgentType::SubManager | AgentType::MainManager => {
                let actors = vec![
                    Planner::ACTOR_ID,
                    SpawnAgent::ACTOR_ID,
                    SendMessage::ACTOR_ID,
                    Complete::ACTOR_ID,
                ];

                actors
            }
            AgentType::Worker => {
                vec![
                    Command::ACTOR_ID,
                    FileReaderActor::ACTOR_ID,
                    EditFile::ACTOR_ID,
                    Planner::ACTOR_ID,
                    MCP::ACTOR_ID,
                    Complete::ACTOR_ID,
                    SendManagerMessage::ACTOR_ID,
                ]
            }
        }
    }

    /// Run the agent - start their actors
    #[tracing::instrument(name = "agent_run", skip(self), fields(agent_id = %self.scope, type = ?self.r#type, role = %self.role))]
    pub async fn run(self) {
        // Create shared file reader
        let file_reader = Arc::new(Mutex::new(FileReader::default()));

        match &self.r#type {
            AgentType::SubManager | AgentType::MainManager => {
                let config = if self.r#type == AgentType::MainManager {
                    self.config.hive.main_manager_model.clone()
                } else {
                    self.config.hive.sub_manager_model.clone()
                };

                // Managers only get planning and agent management tools
                Assistant::new(
                    config,
                    self.tx.clone(),
                    self.scope.clone(),
                    self.parent_scope.clone(),
                    self.get_required_actors(),
                    self.task_description.clone(),
                    Some(self.role.clone()),
                    self.config.whitelisted_commands.clone(),
                    Some(file_reader.clone()),
                )
                .run();
                SendMessage::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
                Wait::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
                Planner::new(
                    self.config.clone(),
                    self.tx.clone(),
                    self.scope.clone(),
                    self.r#type,
                    Some(self.parent_scope.clone()),
                )
                .run();
                SpawnAgent::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();

                Complete::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
            }
            AgentType::Worker => {
                // Workers get all execution tools
                Assistant::new(
                    self.config.hive.worker_model.clone(),
                    self.tx.clone(),
                    self.scope.clone(),
                    self.parent_scope.clone(),
                    self.get_required_actors(),
                    self.task_description.clone(),
                    Some(self.role.clone()),
                    self.config.whitelisted_commands.clone(),
                    Some(file_reader.clone()),
                )
                .run();
                Wait::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
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
                Planner::new(
                    self.config.clone(),
                    self.tx.clone(),
                    self.scope.clone(),
                    self.r#type,
                    Some(self.parent_scope.clone()),
                )
                .run();
                MCP::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
                Complete::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();

                SendManagerMessage::new(
                    self.config.clone(),
                    self.tx.clone(),
                    self.scope.clone(),
                    self.parent_scope.clone(),
                )
                .run();
            }
        }
    }
}

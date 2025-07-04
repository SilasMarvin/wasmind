use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};
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

/// Agent builder
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
    pub actors: BTreeSet<&'static str>,
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
            actors: BTreeSet::new(),
        }
    }

    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    pub fn with_task(mut self, task: String) -> Self {
        self.task_description = Some(task);
        self
    }

    pub fn with_actors(mut self, actors: impl Into<BTreeSet<&'static str>>) -> Self {
        self.actors = actors.into();
        self
    }

    /// Run the agent - start their actors
    #[tracing::instrument(name = "agent_run", skip(self), fields(agent_id = %self.scope, type = ?self.r#type, role = %self.role))]
    pub fn run(self) {
        let config = match self.r#type {
            AgentType::MainManager => self.config.hive.main_manager_model.clone(),
            AgentType::SubManager => self.config.hive.sub_manager_model.clone(),
            AgentType::Worker => self.config.hive.worker_model.clone(),
        };

        let file_reader = Arc::new(Mutex::new(FileReader::default()));

        Assistant::new(
            config,
            self.tx.clone(),
            self.scope.clone(),
            self.parent_scope.clone(),
            self.actors.clone(),
            self.task_description.clone(),
            Some(self.role.clone()),
            self.config.whitelisted_commands.clone(),
            Some(file_reader.clone()),
        )
        .run();

        if self.actors.contains(Planner::ACTOR_ID) {
            Planner::new(
                self.config.clone(),
                self.tx.clone(),
                self.scope.clone(),
                self.r#type,
                Some(self.parent_scope.clone()),
            )
            .run();
        }

        if self.actors.contains(FileReaderActor::ACTOR_ID) {
            FileReaderActor::new(
                self.config.clone(),
                self.tx.clone(),
                file_reader.clone(),
                self.scope.clone(),
            )
            .run();
        }
        if self.actors.contains(EditFile::ACTOR_ID) {
            EditFile::new(
                self.config.clone(),
                self.tx.clone(),
                file_reader.clone(),
                self.scope.clone(),
            )
            .run();
        }

        if self.actors.contains(Command::ACTOR_ID) {
            Command::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        }

        if self.actors.contains(SendMessage::ACTOR_ID) {
            SendMessage::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        }

        if self.actors.contains(SendManagerMessage::ACTOR_ID) {
            SendManagerMessage::new(
                self.config.clone(),
                self.tx.clone(),
                self.scope.clone(),
                self.parent_scope.clone(),
            )
            .run();
        }

        if self.actors.contains(SpawnAgent::ACTOR_ID) {
            SpawnAgent::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        }

        if self.actors.contains(Wait::ACTOR_ID) {
            Wait::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        }

        if self.actors.contains(Complete::ACTOR_ID) {
            Complete::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        }

        if self.actors.contains(MCP::ACTOR_ID) {
            MCP::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        }

        // match &self.r#type {
        //     AgentType::SubManager | AgentType::MainManager => {
        //         let config = if self.r#type == AgentType::MainManager {
        //             self.config.hive.main_manager_model.clone()
        //         } else {
        //             self.config.hive.sub_manager_model.clone()
        //         };
        //
        //         Assistant::new(
        //             config,
        //             self.tx.clone(),
        //             self.scope.clone(),
        //             self.parent_scope.clone(),
        //             self.get_required_actors(),
        //             self.task_description.clone(),
        //             Some(self.role.clone()),
        //             self.config.whitelisted_commands.clone(),
        //             None,
        //         )
        //         .run();
        //
        //         SendMessage::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        //         Planner::new(
        //             self.config.clone(),
        //             self.tx.clone(),
        //             self.scope.clone(),
        //             self.r#type,
        //             Some(self.parent_scope.clone()),
        //         )
        //         .run();
        //         SpawnAgent::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        //         Wait::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        //
        //         Complete::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        //     }
        //     AgentType::Worker => {
        //         // Create shared file reader
        //         let file_reader = Arc::new(Mutex::new(FileReader::default()));
        //
        //         Assistant::new(
        //             self.config.hive.worker_model.clone(),
        //             self.tx.clone(),
        //             self.scope.clone(),
        //             self.parent_scope.clone(),
        //             self.get_required_actors(),
        //             self.task_description.clone(),
        //             Some(self.role.clone()),
        //             self.config.whitelisted_commands.clone(),
        //             Some(file_reader.clone()),
        //         )
        //         .run();
        //
        //         SendManagerMessage::new(
        //             self.config.clone(),
        //             self.tx.clone(),
        //             self.scope.clone(),
        //             self.parent_scope.clone(),
        //         )
        //         .run();
        //         Planner::new(
        //             self.config.clone(),
        //             self.tx.clone(),
        //             self.scope.clone(),
        //             self.r#type,
        //             Some(self.parent_scope.clone()),
        //         )
        //         .run();
        //
        //         Command::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        //         FileReaderActor::new(
        //             self.config.clone(),
        //             self.tx.clone(),
        //             file_reader.clone(),
        //             self.scope.clone(),
        //         )
        //         .run();
        //         EditFile::new(
        //             self.config.clone(),
        //             self.tx.clone(),
        //             file_reader,
        //             self.scope.clone(),
        //         )
        //         .run();
        //         MCP::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        //
        //         Complete::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        //     }
        // }
    }
}

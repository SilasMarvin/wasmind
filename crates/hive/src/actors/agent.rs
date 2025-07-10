use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::sync::broadcast::Sender;

use crate::{
    actors::{
        Actor,
        assistant::Assistant,
        tools::{
            command::CommandTool, complete::CompleteTool, edit_file::EditFile,
            file_reader::FileReaderActor, mcp::MCP, planner::Planner,
            send_manager_message::SendManagerMessage, send_message::SendMessage,
            spawn_agent::SpawnAgent,
        },
    },
    config::{ParsedConfig, ParsedModelConfig},
    scope::Scope,
};

use super::{
    ActorMessage, AgentType,
    temporal::{
        check_health::CheckHealthActor,
        tools::{
            flag_issue_for_review::FlagIssueForReviewTool, report_progress_normal::ReportProgressNormal,
        },
    },
    tools::{file_reader::FileReader, wait::WaitTool},
};

/// Response when spawning a new agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpawnedResponse {
    pub agent_id: Scope,
    pub agent_role: String,
}

pub struct TemporalAgent {
    scope: Scope,
    tx: Sender<ActorMessage>,
    pub task_description: String,
    pub parsed_model_config: ParsedModelConfig,
    pub actors: BTreeSet<&'static str>,
    pub og_scope: Scope,
    pub og_parent_scope: Option<Scope>,
    pub role: String,
}

impl TemporalAgent {
    pub fn new(
        tx: Sender<ActorMessage>,
        task_description: String,
        parsed_model_config: ParsedModelConfig,
        og_scope: Scope,
        role: String,
    ) -> Self {
        Self {
            scope: Scope::new(),
            tx,
            task_description,
            parsed_model_config,
            og_scope,
            actors: BTreeSet::new(),
            og_parent_scope: None,
            role,
        }
    }

    pub fn with_actors(mut self, actors: impl Into<BTreeSet<&'static str>>) -> Self {
        self.actors = actors.into();
        self
    }

    pub fn with_og_parent_scope(mut self, scope: Scope) -> Self {
        self.og_parent_scope = Some(scope);
        self
    }

    /// Run the agent - start their actors
    #[tracing::instrument(name = "temporal_agent_run", skip(self), fields(agent_id = %self.scope))]
    pub fn run(self) {
        Assistant::new(
            self.parsed_model_config,
            self.tx.clone(),
            self.scope.clone(),
            Scope::new(), // Temporal agent's don't care about parent scopes so we set it randomly
            self.actors.clone(),
            Some(self.task_description.clone()),
            self.role.clone(),
            vec![],
            None,
        )
        .run();

        if self.actors.contains(FlagIssueForReviewTool::ACTOR_ID) {
            FlagIssueForReviewTool::new(
                self.tx.clone(),
                self.scope.clone(),
                self.og_scope,
                self.og_parent_scope
                    .expect("`og_parent_scope` is required for FlagIssueForReview temporal tool"),
            )
            .run();
        }

        if self.actors.contains(ReportProgressNormal::ACTOR_ID) {
            ReportProgressNormal::new(self.tx.clone(), self.scope.clone()).run();
        }
    }
}

/// Agent builder
/// TODO: Clean this up we shouldn't have to take in the config and parsed model config
pub struct Agent {
    tx: Sender<ActorMessage>,
    pub r#type: AgentType,
    pub config: ParsedConfig,
    pub task_description: Option<String>,
    pub role: String,
    pub parent_scope: Scope,
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
        let file_reader = Arc::new(Mutex::new(FileReader::default()));

        let parsed_model_config = match self.r#type {
            AgentType::MainManager => self.config.hive.main_manager_model.clone(),
            AgentType::SubManager => self.config.hive.sub_manager_model.clone(),
            AgentType::Worker => self.config.hive.worker_model.clone(),
        };

        Assistant::new(
            parsed_model_config,
            self.tx.clone(),
            self.scope.clone(),
            self.parent_scope.clone(),
            self.actors.clone(),
            self.task_description.clone(),
            self.role,
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

        if self.actors.contains(CommandTool::ACTOR_ID) {
            CommandTool::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
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

        if self.actors.contains(WaitTool::ACTOR_ID) {
            WaitTool::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        }

        if self.actors.contains(CompleteTool::ACTOR_ID) {
            CompleteTool::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        }

        if self.actors.contains(MCP::ACTOR_ID) {
            MCP::new(self.config.clone(), self.tx.clone(), self.scope.clone()).run();
        }

        // Temporal actors

        if self.actors.contains(CheckHealthActor::ACTOR_ID) {
            CheckHealthActor::new(
                self.config.clone(),
                self.tx.clone(),
                self.scope.clone(),
                self.parent_scope.clone(),
                Duration::from_secs(30),
            )
            .run();
        }
    }
}

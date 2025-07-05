use crate::llm_client::ChatRequest;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

use crate::{
    actors::{Actor, ActorMessage, AssistantRequest, agent::TemporalAgent},
    config::ParsedConfig,
    scope::Scope,
};

use super::tools::{
    flag_issue_for_review::FlagIssueForReview, report_progress_normal::ReportProgressNormal,
};

pub struct CheckHealthActor {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)]
    config: ParsedConfig,
    scope: Scope,
    parent_scope: Scope,
    check_interval: Duration,
    last_check: Instant,
    last_assistant_request: Option<AssistantRequest>,
}

impl CheckHealthActor {
    pub fn new(
        config: ParsedConfig,
        tx: broadcast::Sender<ActorMessage>,
        scope: Scope,
        parent_scope: Scope,
        check_interval: Duration,
    ) -> Self {
        Self {
            config,
            tx,
            scope,
            parent_scope,
            check_interval,
            last_check: Instant::now(),
            last_assistant_request: None,
        }
    }

    pub fn handle_assistant_request(&mut self, assistant_request: AssistantRequest) {
        self.last_assistant_request = Some(assistant_request.clone());
        if self.last_check.elapsed() >= self.check_interval {
            let parsed_model_config = self
                .config
                .hive
                .temporal
                .check_health
                .as_ref()
                .unwrap_or(&self.config.hive.worker_model)
                .clone();

            // TODO: Improve how are generating the transcript - maybe we can shorten it / make it cleaner
            let request = ChatRequest {
                model: parsed_model_config.model_name.clone(),
                messages: assistant_request.messages.clone(),
                tools: Some(assistant_request.tools.clone()),
            };
            let task = format!(
                "Analyze the folllowing transcript:\n<transcript>{}</transcript>",
                serde_json::to_string_pretty(&request).unwrap()
            );

            TemporalAgent::new(
                self.tx.clone(),
                task,
                parsed_model_config,
                self.scope.clone(),
            )
            .with_actors([FlagIssueForReview::ACTOR_ID, ReportProgressNormal::ACTOR_ID])
            .with_og_parent_scope(self.parent_scope)
            .run();

            self.last_check = Instant::now();
        }
    }
}

#[async_trait::async_trait]
impl Actor for CheckHealthActor {
    const ACTOR_ID: &'static str = "temporal_worker_health";

    fn get_scope(&self) -> &Scope {
        &self.scope
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        match message.message {
            crate::actors::Message::AssistantRequest(assistant_request) => {
                self.handle_assistant_request(assistant_request);
            }
            _ => (),
        }
    }
}

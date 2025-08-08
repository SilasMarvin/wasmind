use crate::llm_client::ChatRequest;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::{
    actors::{Actor, ActorContext, ActorMessage, AssistantChatState, agent::TemporalAgent},
    config::ParsedConfig,
    scope::Scope,
};

use super::tools::{
    flag_issue_for_review::FlagIssueForReviewTool, report_progress_normal::ReportProgressNormal,
};

#[derive(hive_macros::ActorContext)]
pub struct CheckHealthActor {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)]
    config: ParsedConfig,
    scope: Scope,
    parent_scope: Scope,
    check_interval: Duration,
    last_check: Instant,
    last_assistant_request: Option<AssistantChatState>,
    health_check_task: Option<JoinHandle<()>>,
    last_spawned_time: Option<Instant>,
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
            health_check_task: None,
            last_spawned_time: None,
        }
    }

    pub fn handle_assistant_request(&mut self, assistant_request: AssistantChatState) {
        self.last_assistant_request = Some(assistant_request);
        
        let sleep_duration = if let Some(last_spawned) = self.last_spawned_time {
            let elapsed = last_spawned.elapsed();
            if elapsed < self.check_interval {
                self.check_interval - elapsed
            } else {
                Duration::from_secs(0)
            }
        } else {
            self.check_interval
        };
        
        if let Some(handle) = self.health_check_task.take() {
            handle.abort();
        }
        
        self.spawn_health_check_task(sleep_duration);
        self.last_spawned_time = Some(Instant::now());
    }
    
    fn spawn_health_check_task(&mut self, sleep_duration: Duration) {
        if let Some(assistant_request) = self.last_assistant_request.clone() {
            let tx = self.tx.clone();
            let config = self.config.clone();
            let scope = self.scope.clone();
            let parent_scope = self.parent_scope.clone();
            
            let handle = tokio::spawn(async move {
                tokio::time::sleep(sleep_duration).await;
                
                let parsed_model_config = config.hive.temporal.check_health.clone();
                
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
                    tx,
                    task,
                    parsed_model_config,
                    scope,
                    "Expert LLM Assistant Health Checker".to_string(),
                )
                .with_actors([
                    FlagIssueForReviewTool::ACTOR_ID,
                    ReportProgressNormal::ACTOR_ID,
                ])
                .with_og_parent_scope(parent_scope)
                .run();
            });
            
            self.health_check_task = Some(handle);
            self.last_check = Instant::now();
        }
    }
}

#[async_trait::async_trait]
impl Actor for CheckHealthActor {
    const ACTOR_ID: &'static str = "temporal_worker_health";

    async fn handle_message(&mut self, message: ActorMessage) {
        match message.message {
            crate::actors::Message::AssistantRequest(assistant_request) => {
                self.handle_assistant_request(assistant_request);
            }
            _ => (),
        }
    }
}

use std::time::{Duration, Instant};

use tokio::sync::broadcast;

use crate::{
    actors::{Actor, ActorMessage, AssistantRequest},
    config::ParsedModelConfig,
    scope::Scope,
};

pub struct WorkerHealthActor {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)]
    config: ParsedModelConfig,
    scope: Scope,
    check_interval: Duration,
    last_check: Instant,
    last_assistant_request: Option<AssistantRequest>,
}

impl WorkerHealthActor {
    pub fn new(
        tx: broadcast::Sender<ActorMessage>,
        config: ParsedModelConfig,
        scope: Scope,
        check_interval: Duration,
    ) -> Self {
        Self {
            tx,
            config,
            scope,
            check_interval,
            last_check: Instant::now(),
            last_assistant_request: None,
        }
    }

    pub fn handle_assistant_request(&mut self, assistant_request: AssistantRequest) {
        self.last_assistant_request = Some(assistant_request.clone());
        if self.last_check.elapsed() >= self.check_interval {
            self.last_check = Instant::now();
        }
    }
}

#[async_trait::async_trait]
impl Actor for WorkerHealthActor {
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

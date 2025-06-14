use tokio::sync::broadcast::{Receiver, Sender};
use uuid::Uuid;

use crate::config::ParsedConfig;

use super::{Actor, ActorMessage};

pub struct TuiActor {
    #[allow(dead_code)]
    config: ParsedConfig,
    tx: Sender<ActorMessage>,
    scope: Uuid,
}

impl TuiActor {
    pub fn new(config: ParsedConfig, tx: Sender<ActorMessage>, scope: Uuid) -> Self {
        Self { config, tx, scope }
    }
}

#[async_trait::async_trait]
impl Actor for TuiActor {
    const ACTOR_ID: &'static str = "tui";

    fn get_scope(&self) -> &Uuid {
        &self.scope
    }

    fn get_tx(&self) -> Sender<ActorMessage> {
        todo!()
    }

    fn get_rx(&self) -> Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        todo!()
    }
}

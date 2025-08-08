use hive_actor_loader::LoadedActor;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::broadcast;

mod manager;

// Re-exports for convenience
pub use manager::exports::hive::actor::actor::MessageEnvelope;

use crate::{context::HiveContext, scope::Scope};

pub trait ActorExecutor: Send + Sync {
    fn actor_id(&self) -> &str;

    fn logical_name(&self) -> &str;

    fn auto_spawn(&self) -> bool;

    fn required_spawn_with(&self) -> Vec<&str>;

    fn run(
        &self,
        scope: Scope,
        tx: broadcast::Sender<MessageEnvelope>,
        rx: broadcast::Receiver<MessageEnvelope>,
        context: Arc<HiveContext>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

impl ActorExecutor for LoadedActor {
    fn actor_id(&self) -> &str {
        &self.id
    }

    fn logical_name(&self) -> &str {
        &self.name
    }

    fn auto_spawn(&self) -> bool {
        self.auto_spawn
    }

    fn required_spawn_with(&self) -> Vec<&str> {
        self.required_spawn_with
            .iter()
            .map(|x| x.as_str())
            .collect()
    }

    fn run(
        &self,
        scope: Scope,
        tx: broadcast::Sender<MessageEnvelope>,
        rx: broadcast::Receiver<MessageEnvelope>,
        context: Arc<HiveContext>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let id = self.id.clone();
        let wasm = self.wasm.clone();
        let config = self.config.clone();

        Box::pin(async move {
            let manager = manager::Manager::new(id, &wasm, scope, tx, rx, context, config).await;
            manager.run();
        })
    }
}

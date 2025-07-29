pub mod agent;
mod manager;

use hive_actor_loader::LoadedActor;
// Re-exports for convenience
pub use manager::exports::hive::actor::actor::MessageEnvelope;
use tokio::sync::broadcast;

use crate::scope::Scope;

pub trait ActorExecutor {
    fn actor_id(&self) -> &str;

    fn run(
        self,
        scope: Scope,
        tx: broadcast::Sender<MessageEnvelope>,
    ) -> impl std::future::Future<Output = ()> + Send
    where
        Self: Sized;
}

impl ActorExecutor for LoadedActor {
    fn actor_id(&self) -> &str {
        &self.id
    }

    async fn run(self, scope: Scope, tx: broadcast::Sender<MessageEnvelope>)
    where
        Self: Sized,
    {
        let manager = manager::Manager::new(self.id, &self.wasm, scope, tx, self.config).await;
        manager.run();
    }
}

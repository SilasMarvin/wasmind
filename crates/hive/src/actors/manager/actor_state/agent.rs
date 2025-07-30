use crate::actors::manager::hive::actor::agent;

use super::ActorState;

impl agent::Host for ActorState {
    async fn spawn_agent(&mut self, actor_ids: Vec<String>) -> Result<String, String> {
        // Convert Vec<String> to Vec<&str> for the spawn_agent call
        let actor_refs: Vec<&str> = actor_ids.iter().map(|s| s.as_str()).collect();
        
        // Call spawn_agent on the context
        match self.context.spawn_agent(&actor_refs).await {
            Ok(scope) => Ok(scope.to_string()),
            Err(e) => Err(format!("Failed to spawn agent: {}", e)),
        }
    }
}
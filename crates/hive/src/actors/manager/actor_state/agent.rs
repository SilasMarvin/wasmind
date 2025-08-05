use crate::actors::manager::hive::actor::agent;

use super::ActorState;

impl agent::Host for ActorState {
    async fn spawn_agent(
        &mut self,
        actor_ids: Vec<String>,
        agent_name: String,
    ) -> Result<String, String> {
        // Convert Vec<String> to Vec<&str> for the spawn_agent call
        let actor_refs: Vec<&str> = actor_ids.iter().map(|s| s.as_str()).collect();

        // Call spawn_agent on the context
        match self
            .context
            .spawn_agent(&actor_refs, agent_name, Some(self.scope.clone()))
            .await
        {
            Ok(scope) => Ok(scope.to_string()),
            Err(e) => Err(format!("Failed to spawn agent: {}", e)),
        }
    }

    async fn get_parent_scope(&mut self) -> Option<String> {
        // Get the parent scope for this actor's scope
        self.context.get_parent_scope(self.scope.clone())
    }

    async fn get_parent_scope_of(&mut self, scope: String) -> Option<String> {
        // Get the parent scope directly
        self.context.get_parent_scope(scope)
    }
}

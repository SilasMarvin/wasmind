use crate::actors::manager::hive::actor::agent;

use super::ActorState;

impl agent::Host for ActorState {
    async fn spawn_agent(
        &mut self,
        actor_ids: Vec<String>,
        agent_name: String,
    ) -> Result<String, String> {
        match self
            .context
            .spawn_agent(&actor_ids, agent_name, Some(self.scope.clone()))
            .await
        {
            Ok(scope) => Ok(scope.to_string()),
            Err(e) => Err(format!("Failed to spawn agent: {e}")),
        }
    }

    async fn get_parent_scope(&mut self) -> Option<String> {
        self.context.get_parent_scope(self.scope.clone())
    }

    async fn get_parent_scope_of(&mut self, scope: String) -> Option<String> {
        self.context.get_parent_scope(scope)
    }
}

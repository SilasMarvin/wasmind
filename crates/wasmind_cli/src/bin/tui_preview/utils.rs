use wasmind::scope::new_scope;
use wasmind_actor_utils::common_messages::{Scope, actors::AgentSpawned};

pub fn create_spawn_agent_message(
    name: &str,
    parent_agent: Option<&Scope>,
) -> (AgentSpawned, Scope) {
    let new_scope = new_scope();
    (
        AgentSpawned {
            agent_id: new_scope.to_string(),
            name: name.to_string(),
            parent_agent: parent_agent.map(|s| s.to_string()),
            actors: vec![],
        },
        new_scope,
    )
}

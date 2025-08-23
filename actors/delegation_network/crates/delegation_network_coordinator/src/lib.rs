use std::{
    collections::{HashMap, HashSet},
    i32,
};

use bindings::{
    exports::wasmind::actor::actor::MessageEnvelope, wasmind::actor::agent::get_parent_scope_of,
};
use delegation_network_common_types::{AgentSpawned, AgentType};
use wasmind_actor_utils::common_messages::assistant::{Status, StatusUpdate};

#[allow(warnings)]
mod bindings;

wasmind_actor_utils::actors::macros::generate_actor_trait!();

#[allow(dead_code)]
struct StoredAgent {
    agent_type: AgentType,
    active_agents: HashSet<String>,
}

#[derive(wasmind_actor_utils::actors::macros::Actor)]
struct DelegationNetworkCoordinator {
    active_agents: HashMap<String, StoredAgent>,
}

impl GeneratedActorTrait for DelegationNetworkCoordinator {
    fn new(scope: String, _config_str: String) -> Self {
        // Assume the scope we are created in is the MainManager's scope
        Self {
            active_agents: HashMap::from([(
                scope,
                StoredAgent {
                    agent_type: AgentType::MainManager,
                    active_agents: HashSet::new(),
                },
            )]),
        }
    }

    fn handle_message(&mut self, message: MessageEnvelope) {
        if let Some(agent_spawned) = Self::parse_as::<AgentSpawned>(&message) {
            self.active_agents.insert(
                agent_spawned.agent_id.clone(),
                StoredAgent {
                    agent_type: agent_spawned.agent_type,
                    active_agents: HashSet::new(),
                },
            );

            if let Some(parent) = get_parent_scope_of(&agent_spawned.agent_id) {
                if let Some(parent_agent) = self.active_agents.get_mut(&parent) {
                    parent_agent
                        .active_agents
                        .insert(agent_spawned.agent_id.clone());
                }
            }
        }

        if let Some(status_update) = Self::parse_as::<StatusUpdate>(&message) {
            match status_update.status {
                Status::Done { .. } => {
                    self.active_agents.remove(&message.from_scope);

                    if let Some(parent_scope) = get_parent_scope_of(&message.from_scope) {
                        if let Some(active_agent) = self.active_agents.get_mut(&parent_scope) {
                            active_agent.active_agents.remove(&message.from_scope);
                        }
                    }
                }
                _ => {}
            }
        }

        // Do something here someday?
    }

    fn destructor(&mut self) {}
}

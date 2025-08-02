use std::{
    collections::{HashMap, HashSet},
    i32,
};

use bindings::exports::hive::actor::actor::MessageEnvelope;
use hive_actor_utils::{common_messages, messages::Message};
use serde::{Deserialize, Serialize};

#[allow(warnings)]
mod bindings;

hive_actor_utils::actors::macros::generate_actor_trait!();

#[derive(Clone, Copy, Deserialize, Serialize)]
pub enum AgentType {
    MainManager,
    SubManager,
    Worker,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct AgentSpawned {
    agent_type: AgentType,
    agent_spawned_message: common_messages::actors::AgentSpawned,
}

impl Message for AgentSpawned {
    const MESSAGE_TYPE: &str = "delegation_network.delegation_network_coordinator.AgentSpawned";
}

struct StoredAgent {
    agent_type: AgentType,
    active_agents: HashSet<String>,
}

#[derive(hive_actor_utils::actors::macros::Actor)]
struct DelegationNetworkCoordinator {
    active_agents: HashMap<String, StoredAgent>,
}

impl GeneratedActorTrait for DelegationNetworkCoordinator {
    fn new(_scope: String, _config_str: String) -> Self {
        Self {
            active_agents: HashMap::new(),
        }
    }

    fn handle_message(&mut self, message: MessageEnvelope) {
        if let Some(agent_spawned) = Self::parse_as::<AgentSpawned>(&message) {
            self.active_agents.insert(
                agent_spawned.agent_spawned_message.agent_id,
                StoredAgent {
                    agent_type: agent_spawned.agent_type,
                    active_agents: HashSet::new(),
                },
            );
        }
    }

    fn destructor(&mut self) {}
}

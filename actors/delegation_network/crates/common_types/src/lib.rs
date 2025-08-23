use wasmind_actor_utils::messages::Message;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum AgentType {
    MainManager,
    SubManager,
    Worker,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct AgentSpawned {
    pub agent_type: AgentType,
    pub agent_id: String,
}

impl Message for AgentSpawned {
    const MESSAGE_TYPE: &str = "delegation_network.delegation_network_coordinator.AgentSpawned";
}
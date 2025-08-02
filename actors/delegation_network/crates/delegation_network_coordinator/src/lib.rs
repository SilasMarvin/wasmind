use std::{
    collections::{HashMap, HashSet},
    i32,
};

use bindings::{
    exports::hive::actor::actor::MessageEnvelope, hive::actor::agent::get_parent_scope_of,
};
use hive_actor_utils::{
    common_messages::{
        assistant::{AddMessage, Status, StatusUpdate, WaitReason},
        tools::ExecuteTool,
    },
    llm_client_types::{ChatMessage, SystemChatMessage},
    messages::Message,
};
use serde::{Deserialize, Serialize};

#[allow(warnings)]
mod bindings;

hive_actor_utils::actors::macros::generate_actor_trait!();

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
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
                agent_spawned.agent_id.clone(),
                StoredAgent {
                    agent_type: agent_spawned.agent_type,
                    active_agents: HashSet::new(),
                },
            );

            if let Some(parent) = get_parent_scope_of(&agent_spawned.agent_id)
                && let Some(parent_agent) = self.active_agents.get_mut(&parent)
            {
                parent_agent.active_agents.insert(agent_spawned.agent_id);
            }
        }

        // Things we want to monitor for and not allow:
        // 1. Workers and SubManagers waiting for UserInput
        // 2. Managers using the Wait tool when they have no active agents

        if let Some(status_update) = Self::parse_as::<StatusUpdate>(&message) {
            match status_update.status {
                Status::Wait {
                    reason: WaitReason::WaitingForUserInput,
                } => {
                    // Don't allow workers to wait on UserInput
                    if let Some(active_agent) = self.active_agents.get(&message.from_actor_id)
                        && active_agent.agent_type == AgentType::Worker
                    {
                        let _ = Self::broadcast_common_message(AddMessage {
                            agent: message.from_actor_id.clone(),
                            message: ChatMessage::System(SystemChatMessage {
                                content: "ERROR: You are an Agent in the HIVE system. You must use a tool with every interaction. If you are stuck send a message to your manager. If you are done, use the complete tool.".to_string(),
                            }),
                        });
                    }
                }
                Status::Done { .. } => {
                    self.active_agents.remove(&message.from_actor_id);
                    if let Some(parent_scope) = get_parent_scope_of(&message.from_actor_id)
                        && let Some(active_agent) = self.active_agents.get_mut(&parent_scope)
                    {
                        active_agent.active_agents.remove(&message.from_actor_id);
                    }
                }
                _ => (),
            }
        }

        if let Some(execute_tool_call) = Self::parse_as::<ExecuteTool>(&message)
            && execute_tool_call.tool_call.function.name == "wait"
            && let Some(active_agent) = self.active_agents.get(&message.from_actor_id)
            && active_agent.agent_type == AgentType::SubManager
            && active_agent.active_agents.is_empty()
        {
            let _ = Self::broadcast_common_message(AddMessage {
                agent: message.from_actor_id.clone(),
                message: ChatMessage::System(SystemChatMessage {
                    content: "ERROR: You have no active spawned agents. Waiting will do nothing! Do ANYTHING else!".to_string(),
                }),
            });
        }
    }

    fn destructor(&mut self) {}
}

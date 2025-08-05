use std::{
    collections::{HashMap, HashSet},
    i32,
};

use bindings::{
    exports::hive::actor::actor::MessageEnvelope,
    hive::actor::agent::get_parent_scope_of,
    hive::actor::logger::{LogLevel, log},
};
use delegation_network_common_types::{AgentSpawned, AgentType};
use hive_actor_utils::{
    STARTING_SCOPE,
    common_messages::{
        assistant::{AddMessage, Status, StatusUpdate, WaitReason},
        tools::ExecuteTool,
    },
    llm_client_types::{ChatMessage, SystemChatMessage},
};

#[allow(warnings)]
mod bindings;

hive_actor_utils::actors::macros::generate_actor_trait!();

struct StoredAgent {
    agent_type: AgentType,
    active_agents: HashSet<String>,
}

#[derive(hive_actor_utils::actors::macros::Actor)]
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

        // Don't let managers use the `wait` tool if they have no acive agents to wait on
        if let Some(execute_tool_call) = Self::parse_as::<ExecuteTool>(&message) {
            if execute_tool_call.tool_call.function.name == "wait" {
                if let Some(active_agent) = self.active_agents.get(&message.from_scope) {
                    if active_agent.agent_type == AgentType::SubManager
                        && active_agent.active_agents.is_empty()
                    {
                        let _ = Self::broadcast_common_message(AddMessage {
                            agent: message.from_scope.clone(),
                            message: ChatMessage::System(SystemChatMessage {
                                content: "ERROR: You have no active spawned agents. Waiting will do nothing! Do ANYTHING else!".to_string(),
                            }),
                        });
                    }
                }
            }
        }
    }

    fn destructor(&mut self) {}
}

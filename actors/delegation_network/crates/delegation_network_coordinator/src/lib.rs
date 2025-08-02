use std::i32;

use bindings::exports::hive::actor::actor::MessageEnvelope;
use hive_actor_utils::common_messages::assistant::{SystemPromptContent, SystemPromptContribution};
use serde::Deserialize;

#[allow(warnings)]
mod bindings;

const DEFAULT_IDENTITY_PROMPT: &str = r#"You are the HIVE Delegation Network Coordinator. Your role is to facilitate the creation and management of a hierarchical agent network to accomplish complex tasks."#;

#[derive(Debug, Clone, Deserialize)]
pub struct DelegationNetworkCoordinatorConfig {
    /// Optional custom system prompt for this coordinator
    pub system_prompt: Option<String>,
}

hive_actor_utils::actors::macros::generate_actor_trait!();

#[derive(hive_actor_utils::actors::macros::Actor)]
struct DelegationNetworkCoordinator {}

impl GeneratedActorTrait for DelegationNetworkCoordinator {
    fn new(scope: String, config_str: String) -> Self {
        let config: DelegationNetworkCoordinatorConfig =
            toml::from_str(&config_str).unwrap_or_else(|_| DelegationNetworkCoordinatorConfig {
                system_prompt: None,
            });

        // Set up the system prompt that defines the coordinator's role
        let system_prompt = config
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_IDENTITY_PROMPT);

        Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "hive:delegation_network_coordinator:identity".to_string(),
            content: SystemPromptContent::Text(system_prompt.to_string()),
            priority: i32::MAX,
            section: Some("IDENTITY".to_string()),
        })
        .unwrap();

        Self {}
    }

    fn handle_message(&mut self, _message: MessageEnvelope) {}

    fn destructor(&mut self) {}
}
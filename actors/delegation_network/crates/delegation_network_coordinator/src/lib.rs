use std::i32;

use bindings::exports::hive::actor::actor::MessageEnvelope;
use hive_actor_utils::common_messages::assistant::{SystemPromptContent, SystemPromptContribution};
use serde::Deserialize;

#[allow(warnings)]
mod bindings;

#[derive(Debug, Clone, Deserialize)]
pub struct DelegationNetworkCoordinatorConfig {}

hive_actor_utils::actors::macros::generate_actor_trait!();

#[derive(hive_actor_utils::actors::macros::Actor)]
struct DelegationNetworkCoordinator {
    scope: String,
}

impl GeneratedActorTrait for DelegationNetworkCoordinator {
    fn new(scope: String, config_str: String) -> Self {
        let config: DelegationNetworkCoordinatorConfig =
            toml::from_str(&config_str).expect("Failed to parse assistant config");

        Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "delegation_network.identity".to_string(),
            content: SystemPromptContent::Text("You are the HIVE main manager".to_string()),
            priority: i32::MAX,
            section: Some("IDENTITY".to_string()),
        })
        .unwrap();

        Self { scope }
    }

    fn handle_message(&mut self, message: MessageEnvelope) {}

    fn destructor(&mut self) {}
}

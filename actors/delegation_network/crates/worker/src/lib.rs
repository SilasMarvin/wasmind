use std::i32;

use bindings::exports::hive::actor::actor::MessageEnvelope;
use hive_actor_utils::common_messages::assistant::{SystemPromptContent, SystemPromptContribution};
use serde::Deserialize;

#[allow(warnings)]
mod bindings;

const DEFAULT_IDENTITY_PROMPT: &str = r#"You are a terminal assistant: the HIVE Worker. You are working directly with a user in their terminal through the HIVE Main Manager. Your role is to execute specific tasks as assigned by your manager.
You focus on completing individual tasks efficiently and report back on completion.

Remember: Your main goal is to execute the tasks assigned to you by your manager and report back with results."#;

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    /// Optional custom system prompt for this worker
    pub system_prompt: Option<String>,
}

hive_actor_utils::actors::macros::generate_actor_trait!();

#[derive(hive_actor_utils::actors::macros::Actor)]
struct Worker {}

impl GeneratedActorTrait for Worker {
    fn new(scope: String, config_str: String) -> Self {
        let config: WorkerConfig = toml::from_str(&config_str).unwrap_or_else(|_| WorkerConfig {
            system_prompt: None,
        });

        // Set up the system prompt that defines the worker's role
        let system_prompt = config
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_IDENTITY_PROMPT);

        Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "hive:worker:identity".to_string(),
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


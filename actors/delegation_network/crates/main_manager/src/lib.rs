use std::i32;

use bindings::exports::hive::actor::actor::MessageEnvelope;
use hive_actor_utils::common_messages::assistant::{SystemPromptContent, SystemPromptContribution};
use serde::Deserialize;

#[allow(warnings)]
mod bindings;

const DEFAULT_IDENTITY_PROMPT: &str = r#"You are a terminal assistant: the HIVE Main Manager. You are working directly with a user in their terminal. Your role is to plan and delegate tasks to acomplish the users request - you do not execute tasks yourself. 
You break down complex objectives into manageable tasks and delegate them to appropriate agents (either sub-managers or workers). 
You coordinate the overall strategy and ensure all parts of the project work together harmoniously. 
Use your tools to spawn agents, communicate with them, create plans, and coordinate timing.

Remeber: Your main goal is to satisfy the user's request.

NOTE: The user may say things like: "What are the contents of Cargo.toml" -- the user is assuming you know the current directory they are in and will use the spawn_agent tool to explore the file"#;

#[derive(Debug, Clone, Deserialize)]
pub struct MainManagerConfig {
    /// Optional custom system prompt for this manager
    pub system_prompt: Option<String>,
}

hive_actor_utils::actors::macros::generate_actor_trait!();

#[derive(hive_actor_utils::actors::macros::Actor)]
struct MainManager {
    scope: String,
    config: MainManagerConfig,
}

impl GeneratedActorTrait for MainManager {
    fn new(scope: String, config_str: String) -> Self {
        let config: MainManagerConfig =
            toml::from_str(&config_str).unwrap_or_else(|_| MainManagerConfig {
                system_prompt: None,
            });

        // Set up the system prompt that defines the main manager's role
        let system_prompt = config
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_IDENTITY_PROMPT);

        Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "hive:main_manager:identity".to_string(),
            content: SystemPromptContent::Text(system_prompt.to_string()),
            priority: i32::MAX,
            section: Some("IDENTITY".to_string()),
        })
        .unwrap();

        Self { scope, config }
    }

    fn handle_message(&mut self, _message: MessageEnvelope) {
        // Main manager handles incoming messages from subordinates
        // This could include status updates, questions, or completed task reports
        // For now, we'll keep this simple and let the assistant handle message processing
    }

    fn destructor(&mut self) {
        // Clean up any resources when the main manager is destroyed
    }
}

use std::time::{Duration, Instant};

use bindings::wasmind::actor::agent::spawn_agent;
use wasmind_actor_utils::{
    common_messages::assistant::{AddMessage, Request, Response},
    llm_client_types::{ChatMessage, UserChatMessage},
};

#[allow(warnings)]
mod bindings;

#[derive(Debug, serde::Deserialize)]
struct CheckHealthConfig {
    check_interval: u64,
}

wasmind_actor_utils::actors::macros::generate_actor_trait!();

#[derive(wasmind_actor_utils::actors::macros::Actor)]
struct CheckHealthWorker {
    scope: String,
    config: CheckHealthConfig,
    transcript: Vec<ChatMessage>,
    last_check_time: Option<Instant>,
}

impl GeneratedActorTrait for CheckHealthWorker {
    fn new(scope: String, config_str: String) -> Self {
        let config: CheckHealthConfig = toml::from_str(&config_str)
            .expect("Failed to parse check_health config - configuration is required");

        bindings::wasmind::actor::logger::log(
            bindings::wasmind::actor::logger::LogLevel::Info,
            &format!(
                "CheckHealth worker initialized for scope: {} with interval: {}s",
                scope, config.check_interval
            ),
        );

        Self {
            scope,
            config,
            transcript: Vec::new(),
            last_check_time: None,
        }
    }

    fn handle_message(&mut self, message: bindings::exports::wasmind::actor::actor::MessageEnvelope) {
        // Only care about messages from our own scope (the agent we're monitoring)
        if message.from_scope != self.scope {
            return;
        }

        // Store assistant requests from our scope
        if let Some(request) = Self::parse_as::<Request>(&message) {
            self.transcript = request.chat_state.messages.clone();
            // Add system message to transcript
            self.transcript
                .insert(0, ChatMessage::System(request.chat_state.system));
        }

        // When assistant in our scope gets a response, check health
        if let Some(response) = Self::parse_as::<Response>(&message) {
            // Add assistant response to transcript
            self.transcript
                .push(ChatMessage::Assistant(response.message));

            if self.should_check() {
                self.spawn_health_analyzer();
            }
        }
    }

    fn destructor(&mut self) {
        bindings::wasmind::actor::logger::log(
            bindings::wasmind::actor::logger::LogLevel::Info,
            &format!("CheckHealth worker shutting down for scope: {}", self.scope),
        );
    }
}

impl CheckHealthWorker {
    fn should_check(&self) -> bool {
        match self.last_check_time {
            None => true,
            Some(last) => last.elapsed() >= Duration::from_secs(self.config.check_interval),
        }
    }

    fn spawn_health_analyzer(&mut self) {
        let scope = match spawn_agent(
            &vec![
                "check_health_assistant".to_string(),
                "flag_issue".to_string(),
                "report_normal".to_string(),
            ],
            "Check Health",
        ) {
            Ok(scope) => scope,
            Err(e) => {
                bindings::wasmind::actor::logger::log(
                    bindings::wasmind::actor::logger::LogLevel::Info,
                    &format!("Failed to spawn Check Health {e}",),
                );
                return;
            }
        };

        // Format transcript for analysis
        let transcript_str = self.format_transcript();

        // Send initial message with transcript analysis task
        let task_message = AddMessage {
            agent: scope,
            message: ChatMessage::User(UserChatMessage {
                content: format!(
                    "Analyze this agent behavior transcript to determine if the agent is making progress or if it's stuck/looping:\n\n{}\n\nUse flag_issue tool if you detect problems (loops, repeated failures, no progress) or report_normal if the agent appears healthy.",
                    transcript_str
                ),
            }),
        };
        let _ = Self::broadcast_common_message(task_message);

        self.last_check_time = Some(Instant::now());
    }

    fn format_transcript(&self) -> String {
        self.transcript
            .iter()
            .map(|msg| match msg {
                ChatMessage::System(m) => format!("System: {}", m.content),
                ChatMessage::User(m) => format!("User: {}", m.content),
                ChatMessage::Assistant(m) => {
                    let mut output = if let Some(content) = &m.content {
                        format!("Assistant: {}", content)
                    } else {
                        "Assistant: [no content]".to_string()
                    };
                    if let Some(tool_calls) = &m.tool_calls {
                        if !tool_calls.is_empty() {
                            output.push_str("\nTool calls:");
                            for tool_call in tool_calls {
                                output.push_str(&format!(
                                    "\n  - {}: {}",
                                    tool_call.function.name, tool_call.function.arguments
                                ));
                            }
                        }
                    }
                    output
                }
                ChatMessage::Tool(_m) => {
                    // Tool messages have private fields, so we'll just show a placeholder
                    "Tool: [tool response]".to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}


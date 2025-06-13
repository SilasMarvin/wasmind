use genai::chat::{Tool, ToolCall};
use serde::Deserialize;
use tokio::sync::broadcast;
use tracing::info;
use uuid::Uuid;

// Assuming AgentSpawnedResponse is already Serialize.
// It is imported from crate::actors::agent and used with serde_json::to_string in the original code.
use crate::actors::{
    Actor, ActorMessage, AgentMessage, AgentMessageType, AgentTaskStatus, InterAgentMessage,
    Message, ToolCallStatus, ToolCallType, ToolCallUpdate,
    agent::{Agent, AgentSpawnedResponse},
};
use crate::config::ParsedConfig;

pub const TOOL_NAME: &str = "spawn_agents";
pub const TOOL_DESCRIPTION: &str = "Spawns one or more new agents (Worker or Manager), each with a specific task. Spawned agents run independently and report back status updates. Use this tool to delegate work to specialized agents.";
pub const TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "agents_to_spawn": {
            "type": "array",
            "description": "A list of agents to be created (at least one). Each agent in the list will be configured with its own role, task, and type.",
            "minItems": 1,
            "items": {
                "type": "object",
                "properties": {
                    "agent_role": {
                        "type": "string",
                        "description": "The specific role for this agent (e.g., 'Software Engineer', 'QA Tester', 'Project Lead Manager'). This helps define the agent's expertise and focus."
                    },
                    "task_description": {
                        "type": "string",
                        "description": "A clear and concise description of the task or objective assigned to this agent. This is the primary goal the agent will work towards."
                    },
                    "agent_type": {
                        "type": "string",
                        "enum": ["Worker", "Manager"],
                        "description": "Specify 'Worker' if the agent should execute tasks directly. Specify 'Manager' if the agent should delegate or manage tasks, potentially by spawning other agents."
                    }
                },
                "required": ["agent_role", "task_description", "agent_type"]
            }
        },
        "wait": {
            "type": "boolean",
            "description": "Set to true to pause your own operations and await status updates or completion signals from the spawned agent(s) before proceeding. Set to false (default) to continue with other actions immediately after spawning."
        }
    },
    "required": ["agents_to_spawn"]
}"#;

#[derive(Debug, Deserialize)]
struct AgentDefinition {
    agent_role: String,
    task_description: String,
    agent_type: String,
}

#[derive(Debug, Deserialize)]
struct SpawnAgentsInput {
    agents_to_spawn: Vec<AgentDefinition>,
    #[serde(default)]
    wait: bool,
}

/// SpawnAgent tool actor for managers to spawn new agents
pub struct SpawnAgent {
    tx: broadcast::Sender<ActorMessage>,
    config: ParsedConfig,
    scope: Uuid,
}

impl SpawnAgent {
    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Uuid) -> Self {
        Self { tx, config, scope }
    }

    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        if tool_call.fn_name != TOOL_NAME {
            return;
        }

        // Send received status
        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id.clone(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::SpawnAgent,
                friendly_command_display: "Processing request to spawn agents...".to_string(),
            },
        }));

        // Parse input
        let input: SpawnAgentsInput = match serde_json::from_value(tool_call.fn_arguments) {
            Ok(input) => input,
            Err(e) => {
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!("Invalid input schema: {}. Ensure 'agents_to_spawn' is a non-empty array of valid agent definitions.", e))),
                }));
                return;
            }
        };

        // Schema validation (minItems: 1) should ideally catch this, but an explicit check is good.
        if input.agents_to_spawn.is_empty() {
            let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call.call_id,
                status: ToolCallStatus::Finished(Err("No agents specified in 'agents_to_spawn' array. At least one agent must be provided.".to_string())),
            }));
            return;
        }

        // Broadcast the new agent status
        if input.wait {
            self.broadcast(Message::Agent(AgentMessage {
                agent_id: self.scope.clone(),
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status: AgentTaskStatus::Waiting {
                        tool_call_id: tool_call.call_id.clone(),
                    },
                }),
            }))
        }

        let mut spawned_agents_responses: Vec<AgentSpawnedResponse> = Vec::new();
        let mut successfully_spawned_agents_details: Vec<String> = Vec::new();

        for agent_def in input.agents_to_spawn {
            // Create the new agent
            let agent = match agent_def.agent_type.as_str() {
                "Worker" => Agent::new_worker(
                    self.tx.clone(),
                    agent_def.agent_role.clone(),
                    Some(agent_def.task_description.clone()),
                    self.config.clone(),
                    self.scope.clone(),
                ),
                "Manager" => Agent::new_manager(
                    self.tx.clone(),
                    agent_def.agent_role.clone(),
                    Some(agent_def.task_description.clone()),
                    self.config.clone(),
                    self.scope.clone(),
                ),
                _ => {
                    let error_msg = format!(
                        "Invalid agent_type: '{}' for agent role '{}'. Must be 'Worker' or 'Manager'.",
                        agent_def.agent_type, agent_def.agent_role
                    );
                    let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                        call_id: tool_call.call_id.clone(),
                        status: ToolCallStatus::Finished(Err(error_msg)),
                    }));
                    return;
                }
            };

            // Send AgentSpawned message to update system state for this agent
            let _ = self.broadcast(Message::Agent(AgentMessage {
                agent_id: agent.scope.clone(),
                message: AgentMessageType::AgentSpawned {
                    agent_type: agent.r#type,
                    role: agent.role.clone(),
                    task_description: agent_def.task_description.clone(),
                    tool_call_id: tool_call.call_id.clone(),
                },
            }));

            // Collect response for this agent
            spawned_agents_responses.push(AgentSpawnedResponse {
                agent_id: agent.scope.clone(),
                agent_role: agent.role.clone(),
            });
            successfully_spawned_agents_details.push(format!(
                "ID: {}, Role: '{}', Type: {}",
                agent.scope.clone(),
                agent.role.clone(),
                agent_def.agent_type
            ));

            // Spawn the agent task to run asynchronously
            tokio::spawn(async move {
                agent.run().await;
            });
        }

        // All agents defined in the input have been processed and spawn tasks initiated

        info!(
            "Successfully initiated spawning for {} agent(s): [{}]. Wait flag was: {}.",
            spawned_agents_responses.len(),
            successfully_spawned_agents_details.join("; "),
            input.wait
        );

        // Return concise response
        let response = format!(
            "Spawned {} agent{}: {}",
            spawned_agents_responses.len(),
            if spawned_agents_responses.len() == 1 {
                ""
            } else {
                "s"
            },
            spawned_agents_responses
                .iter()
                .map(|r| format!("{} ({})", r.agent_role, r.agent_id))
                .collect::<Vec<_>>()
                .join(", ")
        );

        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(response)),
        }));
    }
}

#[async_trait::async_trait]
impl Actor for SpawnAgent {
    const ACTOR_ID: &'static str = "spawn_agent"; // Internal actor ID, can remain singular

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    fn get_scope(&self) -> &Uuid {
        &self.scope
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        match message.message {
            Message::AssistantToolCall(tool_call) => {
                self.handle_tool_call(tool_call).await;
            }
            _ => {}
        }
    }

    async fn on_start(&mut self) {
        info!("SpawnAgent (spawn_agents tool) actor started"); // Clarified log

        // Send tool availability
        let tool = Tool {
            name: TOOL_NAME.to_string(),                     // Uses updated TOOL_NAME
            description: Some(TOOL_DESCRIPTION.to_string()), // Uses updated TOOL_DESCRIPTION
            schema: Some(
                serde_json::from_str(TOOL_INPUT_SCHEMA)
                    .expect("TOOL_INPUT_SCHEMA must be valid JSON"),
            ), // Uses updated TOOL_INPUT_SCHEMA
        };

        let _ = self.broadcast(Message::ToolsAvailable(vec![tool]));
    }
}

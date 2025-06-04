use genai::chat::{Tool, ToolCall};
use serde::Deserialize;
use tokio::sync::broadcast;
use tracing::info;

use crate::actors::{
    Actor, Message, ToolCallStatus, ToolCallType, ToolCallUpdate,
    agent::{Agent, AgentSpawnedResponse, InterAgentMessage},
};
use crate::config::ParsedConfig;

pub const TOOL_NAME: &str = "spawn_agent_and_assign_task";
pub const TOOL_DESCRIPTION: &str = "Spawn a new agent (Worker or Manager) and assign it a task. The agent will run independently and report back status updates.";
pub const TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "agent_role": {
            "type": "string",
            "description": "The role of the agent to spawn (e.g., 'Software Engineer', 'Project Lead Manager')"
        },
        "task_description": {
            "type": "string",
            "description": "The task or objective to assign to the agent"
        },
        "agent_type": {
            "type": "string",
            "enum": ["Worker", "Manager"],
            "description": "Whether to spawn a Worker agent (executes tasks) or Manager agent (delegates tasks)"
        },
        "wait": {
            "type": "boolean",
            "description": "Whether to wait for the agent to complete before continuing (default: false)"
        }
    },
    "required": ["agent_role", "task_description", "agent_type"]
}"#;

#[derive(Debug, Deserialize)]
struct SpawnAgentInput {
    agent_role: String,
    task_description: String,
    agent_type: String,
    #[serde(default)]
    wait: bool,
}

/// SpawnAgent tool actor for managers to spawn new agents
pub struct SpawnAgent {
    tx: broadcast::Sender<Message>,
    config: ParsedConfig,
    /// Channel to communicate with spawned child agents
    child_tx: broadcast::Sender<InterAgentMessage>,
}

impl SpawnAgent {
    pub fn new_with_channel(
        config: ParsedConfig,
        tx: broadcast::Sender<Message>,
        child_tx: broadcast::Sender<InterAgentMessage>,
    ) -> Self {
        Self {
            tx,
            config,
            child_tx,
        }
    }

    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        if tool_call.fn_name != TOOL_NAME {
            return;
        }

        // Send received status
        let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id.clone(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::MCP, // Using MCP type for now, we can add a new type later
                friendly_command_display: format!("Spawning {} agent", tool_call.fn_name),
            },
        }));

        // Parse input
        let input: SpawnAgentInput = match serde_json::from_value(tool_call.fn_arguments) {
            Ok(input) => input,
            Err(e) => {
                let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!("Invalid input: {}", e))),
                }));
                return;
            }
        };

        // Create the new agent
        let agent = match input.agent_type.as_str() {
            "Worker" => Agent::new_worker(
                input.agent_role.clone(),
                input.task_description.clone(),
                self.config.clone(),
            ),
            "Manager" => Agent::new_manager(
                input.agent_role.clone(),
                input.task_description.clone(),
                self.config.clone(),
            ),
            _ => {
                let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!(
                        "Invalid agent_type: {}. Must be 'Worker' or 'Manager'",
                        input.agent_type
                    ))),
                }));
                return;
            }
        };

        let agent_id = agent.id().clone();
        let task_id = agent.task_id.clone();
        let agent_role = agent.role().to_string();

        // Set up the agent's parent communication channel
        let mut agent = agent;
        agent.parent_tx = Some(self.child_tx.clone());

        // Send AgentSpawned message to update system state
        let _ = self.tx.send(Message::AgentSpawned {
            agent_id: agent_id.clone(),
            agent_role: agent_role.clone(),
            task_id: task_id.clone(),
            task_description: input.task_description.clone(),
        });

        // Create response
        let response = AgentSpawnedResponse {
            agent_id: agent_id.clone(),
            task_id: task_id.clone(),
            agent_role: agent_role.clone(),
        };

        let response_json = serde_json::to_string(&response)
            .unwrap_or_else(|_| format!("Agent spawned with ID: {}", agent_id.0));

        if input.wait {
            // If wait is true, spawn the agent and wait for completion
            let mut child_rx = self.child_tx.subscribe();
            let agent_id_copy = agent_id.clone();
            let task_id_copy = task_id.clone();

            // Spawn the agent
            tokio::spawn(async move {
                agent.run().await;
            });

            // Wait for the agent to complete
            tokio::spawn(async move {
                loop {
                    if let Ok(InterAgentMessage::TaskStatusUpdate {
                        task_id,
                        status,
                        from_agent,
                    }) = child_rx.recv().await
                    {
                        if from_agent == agent_id_copy && task_id == task_id_copy {
                            if let crate::actors::agent::TaskStatus::Done(_result) = status {
                                // Agent completed, we'll handle this in the manager's message loop
                                break;
                            }
                        }
                    }
                }
            });

            // For now, just return that we're waiting
            let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call.call_id,
                status: ToolCallStatus::Finished(Ok(format!(
                    "{}\nWaiting for agent to complete task...",
                    response_json
                ))),
            }));
        } else {
            // Spawn the agent without waiting
            tokio::spawn(async move {
                agent.run().await;
            });

            let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call.call_id,
                status: ToolCallStatus::Finished(Ok(response_json)),
            }));
        }
    }
}

#[async_trait::async_trait]
impl Actor for SpawnAgent {
    const ACTOR_ID: &'static str = "spawn_agent";

    fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self {
        // This shouldn't be called directly, use new_with_channel instead
        let (child_tx, _) = broadcast::channel(1024);
        Self {
            tx,
            config,
            child_tx,
        }
    }

    fn get_rx(&self) -> broadcast::Receiver<Message> {
        self.tx.subscribe()
    }

    fn get_tx(&self) -> broadcast::Sender<Message> {
        self.tx.clone()
    }

    async fn handle_message(&mut self, message: Message) {
        match message {
            Message::AssistantToolCall(tool_call) => {
                self.handle_tool_call(tool_call).await;
            }
            _ => {}
        }
    }

    async fn on_start(&mut self) {
        info!("SpawnAgent tool actor started");

        // Send tool availability
        let tool = Tool {
            name: TOOL_NAME.to_string(),
            description: Some(TOOL_DESCRIPTION.to_string()),
            schema: Some(serde_json::from_str(TOOL_INPUT_SCHEMA).unwrap()),
        };

        let _ = self.tx.send(Message::ToolsAvailable(vec![tool]));
    }
}

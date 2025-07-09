use crate::llm_client::ToolCall;
use serde::Deserialize;
use tokio::sync::broadcast;
use tracing::info;

// Assuming AgentSpawnedResponse is already Serialize.
// It is imported from crate::actors::agent and used with serde_json::to_string in the original code.
use crate::actors::{
    Actor, ActorMessage, AgentMessage, AgentMessageType, AgentType, Message, ToolCallResult, ToolCallStatus,
    ToolCallUpdate, ActorContext,
    agent::{Agent, AgentSpawnedResponse},
    temporal::check_health::CheckHealthActor,
    tools::{
        command::Command, complete::Complete, edit_file::EditFile, file_reader::FileReaderActor,
        mcp::MCP, planner::Planner, send_manager_message::SendManagerMessage,
        send_message::SendMessage, wait::WaitTool,
    },
};
use crate::config::ParsedConfig;
use crate::scope::Scope;

use super::Tool;

const TOOL_NAME: &str = "spawn_agents";
const TOOL_DESCRIPTION: &str = "Spawns one or more new agents (Worker or Manager), each with a specific task. Spawned agents run independently and report back status updates. Use this tool to delegate work to specialized agents.";
const TOOL_INPUT_SCHEMA: &str = r#"{
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
            "description": "If `true` pause and wait for a response from your spawned agents else continue performing actions (default `false`)"
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
pub struct SpawnAgentsInput {
    agents_to_spawn: Vec<AgentDefinition>,
    wait: Option<bool>,
}

/// SpawnAgent tool actor for managers to spawn new agents
#[derive(hive_macros::ActorContext)]
pub struct SpawnAgent {
    tx: broadcast::Sender<ActorMessage>,
    config: ParsedConfig,
    scope: Scope,
}

impl SpawnAgent {
    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self { tx, config, scope }
    }

}

#[async_trait::async_trait]
impl Tool for SpawnAgent {
    const TOOL_NAME: &str = TOOL_NAME;
    const TOOL_DESCRIPTION: &str = TOOL_DESCRIPTION;
    const TOOL_INPUT_SCHEMA: &str = TOOL_INPUT_SCHEMA;

    type Params = SpawnAgentsInput;

    async fn execute_tool_call(&mut self, tool_call: ToolCall, params: Self::Params) {
        // Schema validation (minItems: 1) should ideally catch this, but an explicit check is good.
        if params.agents_to_spawn.is_empty() {
            self.broadcast_finished(
                &tool_call.id,
                ToolCallResult::Err("No agents specified in 'agents_to_spawn' array. At least one agent must be provided.".to_string()),
                None,
            );
            return;
        }

        let mut spawned_agents_responses: Vec<AgentSpawnedResponse> = Vec::new();
        let mut successfully_spawned_agents_details: Vec<String> = Vec::new();

        for agent_def in params.agents_to_spawn {
            // Create the new agent
            let agent = match agent_def.agent_type.as_str() {
                "Worker" => Agent::new(
                    self.tx.clone(),
                    agent_def.agent_role.clone(),
                    Some(agent_def.task_description.clone()),
                    self.config.clone(),
                    self.scope.clone(),
                    AgentType::Worker,
                )
                .with_actors([
                    SendManagerMessage::ACTOR_ID,
                    Planner::ACTOR_ID,
                    Command::ACTOR_ID,
                    FileReaderActor::ACTOR_ID,
                    EditFile::ACTOR_ID,
                    MCP::ACTOR_ID,
                    Complete::ACTOR_ID,
                    CheckHealthActor::ACTOR_ID,
                ]),
                "Manager" => Agent::new(
                    self.tx.clone(),
                    agent_def.agent_role.clone(),
                    Some(agent_def.task_description.clone()),
                    self.config.clone(),
                    self.scope.clone(),
                    AgentType::SubManager,
                )
                .with_actors([
                    SendManagerMessage::ACTOR_ID,
                    SendMessage::ACTOR_ID,
                    Planner::ACTOR_ID,
                    Complete::ACTOR_ID,
                    WaitTool::ACTOR_ID,
                ]),
                _ => {
                    let error_msg = format!(
                        "Invalid agent_type: '{}' for agent role '{}'. Must be 'Worker' or 'Manager'.",
                        agent_def.agent_type, agent_def.agent_role
                    );
                    self.broadcast_finished(
                        &tool_call.id,
                        ToolCallResult::Err(error_msg),
                        None,
                    );
                    return;
                }
            };

            // Send AgentSpawned message to update system state for this agent
            self.broadcast(Message::Agent(AgentMessage {
                agent_id: agent.scope.clone(),
                message: AgentMessageType::AgentSpawned {
                    agent_type: agent.r#type,
                    role: agent.role.clone(),
                    task_description: agent_def.task_description.clone(),
                    tool_call_id: tool_call.id.clone(),
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

            // Spawn the agent task to run
            agent.run();
        }

        info!(
            "Successfully initiated spawning for {} agent(s): [{}]",
            spawned_agents_responses.len(),
            successfully_spawned_agents_details.join("; "),
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

        self.broadcast_finished(
            &tool_call.id,
            ToolCallResult::Ok(response),
            None,
        );
    }
}

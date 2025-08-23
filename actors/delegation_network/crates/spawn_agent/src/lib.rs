use delegation_network_common_types::{AgentSpawned, AgentType};
use serde::Deserialize;
use wasmind_actor_utils::{
    common_messages::{
        assistant::{
            AddMessage, RequestStatusUpdate, Section, Status, SystemPromptContent,
            SystemPromptContribution, WaitReason,
        },
        tools::{ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, UIDisplayInfo},
    },
    llm_client_types::ChatMessage,
    tools,
};

#[allow(warnings)]
mod bindings;

const SPAWN_AGENT_USAGE_GUIDE: &str = r#"<tool name="spawn_agent">Create and Delegate to Specialized Agents

**Purpose**: Create one or more new SubManager or Worker agents to delegate tasks. You can spawn multiple agents in a single call for parallel execution.

**Agent Types and Capabilities (CRITICAL DISTINCTION):**

**1. Worker Agents (`agent_type: "Worker"`)**
   - These are your "hands-on" specialists.
   - **Capabilities**: FULL access to execution tools: command line, file system, network requests, etc.
   - **Use For**: Specific, self-contained tasks like writing code, reading a file, running a test, or fetching data.

**2. SubManager Agents (`agent_type: "SubManager"`)**
   - These are orchestrators, just like you, for managing complex sub-projects.
   - **Capabilities**: They have the SAME tools as you (`spawn_agent`, `planner`). They **CANNOT** directly execute code.
   - **Use For**: Complex, multi-step goals that require their own planning and delegation.

**The `wait` Parameter for Coordination:**

- **`wait: true`**: Use this to run tasks **sequentially**. The system will pause you until the current task (or batch of tasks) is complete. This is essential for preventing conflicts.
- **`wait: false` (Default)**: Use this **with extreme caution** only for tasks that are truly independent.

**CRITICAL: Avoiding Race Conditions and Task Conflicts**

A race condition occurs when agents interfere with each other. This can happen if they modify the same file simultaneously or if one agent depends on another's unfinished work. Since you can spawn multiple agents in a single call, you MUST be vigilant about this.

**RULE**: Only include multiple agents in a single `spawn_agent` call if their tasks are **100% independent** of each other.

**ANTI-PATTERN: Incorrect Parallel Spawning (DO NOT DO THIS)**

- **File Conflict Example:** Spawning two agents to work on the same file in one call. They will overwrite each other's work.
```json
// INCORRECT: Both agents will fight over `utils.py`
{
  "agents_to_spawn": [
    {
      "agent_role": "Refactor Specialist",
      "task_description": "Refactor the calculate() function in utils.py",
      "agent_type": "Worker"
    },
    {
      "agent_role": "Documentation Writer",
      "task_description": "Add docstrings to all functions in utils.py",
      "agent_type": "Worker"
    }
  ]
}
```

- **Dependency Conflict Example:** Spawning an agent to test code that hasn't been written yet in the same call.
```json
// INCORRECT: The testing agent will start immediately and fail because the endpoint doesn't exist.
{
  "agents_to_spawn": [
    {
      "agent_role": "API Developer",
      "task_description": "Create a new /users endpoint in api.py",
      "agent_type": "Worker"
    },
    {
      "agent_role": "QA Engineer",
      "task_description": "Write integration tests for the new /users endpoint",
      "agent_type": "Worker"
    }
  ]
}
```

- **CORRECT PATTERN 1**: Sequential Execution for Dependent Tasks

To handle dependent tasks, you must make separate spawn_agent calls

1. **First Call**: Create the endpoint.
```json
{
  "agents_to_spawn": [{
    "agent_role": "API Developer",
    "task_description": "Create a new /users endpoint in api.py",
    "agent_type": "Worker"
  }],
  "wait": true
}
```

2. After the first agent completes, you are woken up. Now you can spawn the next one.

3. **Second Call**: Test the endpoint that now exists.
```json
{
  "agents_to_spawn": [{
    "agent_role": "QA Engineer",
    "task_description": "Write integration tests for the now-existing /users endpoint",
    "agent_type": "Worker"
  }],
  "wait": true
}
```

**CORRECT PATTERN 2**: Parallel Execution for Independent Tasks

```json
// CORRECT: These tasks are independent and can run in parallel safely.
{
  "agents_to_spawn": [
    {
      "agent_role": "Security Analyst",
      "task_description": "Scan the `./auth-service/` directory for security vulnerabilities.",
      "agent_type": "Worker"
    },
    {
      "agent_role": "Performance Analyst",
      "task_description": "Analyze the `./database-service/` directory for performance bottlenecks.",
      "agent_type": "Worker"
    }
  ],
  "wait": true
}
```

NOTE: The examples above have simplified `task_description`s It is CRITICAL you write highly detailed `task_description`s. They must clearly state everything you want the agent to acomplish. Do NOT expect it to know what you desire unless clearly stated.
</tool>"#;

#[derive(Clone, Deserialize)]
pub struct SpawnAgentConfig {
    pub worker_actors: Vec<String>,
    pub sub_manager_actors: Vec<String>,
}

#[derive(serde::Deserialize)]
struct AgentDefinition {
    agent_role: String,
    task_description: String,
    agent_type: AgentType,
}

#[derive(serde::Deserialize)]
struct SpawnAgentsInput {
    agents_to_spawn: Vec<AgentDefinition>,
    #[serde(
        default,
        deserialize_with = "wasmind_actor_utils::utils::deserialize_flexible_bool"
    )]
    wait: Option<bool>,
}

#[derive(tools::macros::Tool)]
#[tool(
    name = "spawn_agent",
    description = "Spawns one or more new agents (Worker or Manager), each with a specific task. Spawned agents run independently and report back status updates. Use this tool to delegate work to specialized agents.",
    schema = r#"{
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
                            "enum": ["Worker", "Manager", "SubManager"],
                            "description": "Specify 'Worker' if the agent should execute tasks directly. Specify 'Manager' if the agent should delegate or manage tasks, potentially by spawning other agents. Specify 'SubManager' for mid-level management of specific project domains."
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
    }"#
)]
struct SpawnAgentTool {
    scope: String,
    config: SpawnAgentConfig,
}

impl tools::Tool for SpawnAgentTool {
    fn new(scope: String, config: String) -> Self {
        // Broadcast detailed guidance about how to use the spawn_agent tool
        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "spawn_agent:usage_guide".to_string(),
            content: SystemPromptContent::Text(SPAWN_AGENT_USAGE_GUIDE.to_string()),
            priority: 1000,
            section: Some(Section::Tools),
        });

        Self {
            scope,
            config: toml::from_str(&config).expect("Error deserializing config"),
        }
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        let params: SpawnAgentsInput =
            match serde_json::from_str(&tool_call.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    let error_msg = format!("Failed to parse spawn agent parameters: {}", e);
                    let error_result = ToolCallResult {
                        content: error_msg.clone(),
                        ui_display_info: UIDisplayInfo {
                            collapsed: "Parameters: Invalid format".to_string(),
                            expanded: Some(format!(
                                "Error: Failed to parse parameters\n\nDetails: {}",
                                error_msg
                            )),
                        },
                    };
                    self.send_error_result(
                        &tool_call.tool_call.id,
                        &tool_call.originating_request_id,
                        error_result,
                    );
                    return;
                }
            };

        // Validate we have agents to spawn
        if params.agents_to_spawn.is_empty() {
            let error_result = ToolCallResult {
                content: "No agents specified in 'agents_to_spawn' array. At least one agent must be provided.".to_string(),
                ui_display_info: UIDisplayInfo {
                    collapsed: "No agents: Empty list provided".to_string(),
                    expanded: Some("Error: No agents were specified for spawning\n\nAt least one agent must be provided in the agents_to_spawn array.".to_string()),
                },
            };
            self.send_error_result(
                &tool_call.tool_call.id,
                &tool_call.originating_request_id,
                error_result,
            );
            return;
        }

        let mut spawned_agents = Vec::new();
        let mut spawned_agent_ids = Vec::new();

        // Process each agent to spawn
        for agent_def in &params.agents_to_spawn {
            // Determine actors based on agent type
            let actors = match agent_def.agent_type {
                AgentType::Worker => self.config.worker_actors.clone(),
                AgentType::SubManager => self.config.sub_manager_actors.clone(),
                _ => unreachable!(),
            };

            // Use the host's spawn_agent function
            let agent_id = match bindings::wasmind::actor::agent::spawn_agent(
                &actors,
                &agent_def.agent_role,
            ) {
                Ok(scope) => scope,
                Err(e) => {
                    let error_result = ToolCallResult {
                        content: format!("Failed to spawn agent: {}", e),
                        ui_display_info: UIDisplayInfo {
                            collapsed: format!(
                                "Agent spawn: Failed to create {:?}",
                                agent_def.agent_type
                            ),
                            expanded: Some(format!(
                                "Agent Type: {:?}\nError: {}",
                                agent_def.agent_type, e
                            )),
                        },
                    };
                    self.send_error_result(
                        &tool_call.tool_call.id,
                        &tool_call.originating_request_id,
                        error_result,
                    );
                    return;
                }
            };

            // Add task to the agent's system prompt
            let system_prompt_contribution = SystemPromptContribution {
                agent: agent_id.clone(),
                key: "spawn_agent:task".to_string(),
                content: SystemPromptContent::Text(format!(
                    "Your task: {}\nRole: {}",
                    agent_def.task_description, agent_def.agent_role
                )),
                priority: 100, // High priority so it appears early in system prompt
                section: Some(Section::Instructions),
            };

            // Send system prompt contribution
            let _ = Self::broadcast_common_message(system_prompt_contribution);

            // Send initial message from user telling the agent to execute its task
            let task_message = AddMessage {
                agent: agent_id.clone(),
                message: ChatMessage::user(&format!(
                    "You have been assigned the following task: {}\n\nPlease begin working on this task using the tools available to you.",
                    agent_def.task_description
                )),
            };

            // Send the task message
            let _ = Self::broadcast_common_message(task_message);

            // Broadcast the spawned agent message for the DelegationNetwork
            let _ = Self::broadcast_common_message(AgentSpawned {
                agent_type: agent_def.agent_type,
                agent_id: agent_id.clone(),
            });

            spawned_agents.push(format!(
                "{} ({:?}) - ID: {}",
                agent_def.agent_role, agent_def.agent_type, agent_id
            ));
            spawned_agent_ids.push(agent_id);
        }

        // If wait is true, send a status update request to make the agent wait
        if params.wait.unwrap_or(false) {
            let status_update_request = RequestStatusUpdate {
                agent: self.scope.clone(),
                status: Status::Wait {
                    reason: WaitReason::WaitingForAgentCoordination {
                        originating_request_id: tool_call.originating_request_id.clone(),
                        coordinating_tool_name: "spawn_agent".to_string(),
                        target_agent_scope: None,
                        user_can_interrupt: true,
                    },
                },
                originating_request_id: Some(tool_call.originating_request_id.clone()),
            };

            let _ = Self::broadcast_common_message(status_update_request);
        }

        // Create success result
        let success_message = format!(
            "Successfully spawned {} agent{}: {}",
            params.agents_to_spawn.len(),
            if params.agents_to_spawn.len() == 1 {
                ""
            } else {
                "s"
            },
            spawned_agents.join(", ")
        );

        // Build detailed expanded view with agent information
        let mut expanded_content = String::from("Spawned Agents:\n");
        expanded_content.push_str("==================\n");

        for (i, agent_def) in params.agents_to_spawn.iter().enumerate() {
            expanded_content.push_str(&format!(
                "\n{}. {} (Type: {:?})\n   ID: {}\n   Task: {}\n",
                i + 1,
                agent_def.agent_role,
                agent_def.agent_type,
                spawned_agent_ids[i],
                agent_def.task_description
            ));
        }

        let result = ToolCallResult {
            content: success_message.clone(),
            ui_display_info: UIDisplayInfo {
                collapsed: format!(
                    "{} agent{} created: {}",
                    params.agents_to_spawn.len(),
                    if params.agents_to_spawn.len() == 1 {
                        ""
                    } else {
                        "s"
                    },
                    spawned_agents.join(", ")
                ),
                expanded: Some(expanded_content),
            },
        };

        self.send_success_result(
            &tool_call.tool_call.id,
            &tool_call.originating_request_id,
            result,
        );
    }
}

impl SpawnAgentTool {
    fn send_error_result(
        &self,
        tool_call_id: &str,
        originating_request_id: &str,
        error_result: ToolCallResult,
    ) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: originating_request_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(error_result),
            },
        };
        let _ = Self::broadcast_common_message(update);
    }

    fn send_success_result(
        &self,
        tool_call_id: &str,
        originating_request_id: &str,
        result: ToolCallResult,
    ) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: originating_request_id.to_string(),
            status: ToolCallStatus::Done { result: Ok(result) },
        };
        let _ = Self::broadcast_common_message(update);
    }
}

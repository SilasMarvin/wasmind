use delegation_network_common_types::{AgentSpawned, AgentType};
use hive_actor_utils::{
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
use serde::Deserialize;

#[allow(warnings)]
mod bindings;

const SPAWN_AGENT_USAGE_GUIDE: &str = r#"## spawn_agent Tool - Create Specialized Agents

**Purpose**: Create new Manager or Worker agents with specific roles and tasks.

**Agent Capabilities**: All spawned agents have FULL access to:
- Command line interface and terminal operations  
- File system operations (read, write, create, delete)
- Network operations and web requests
- Programming languages (Python, JavaScript, Rust, etc.)
- System administration tasks
- Database operations
- API integrations
- Essentially ANY computer operation available

**When to Use**:
- ✅ Need to delegate a specific task or project
- ✅ Task requires specialized expertise (coding, research, analysis, etc.)
- ✅ Want parallel execution of multiple tasks
- ✅ Need a sub-manager to coordinate complex multi-step work

**Task Description Guidelines**:
Be extremely specific about what you want accomplished. Include:
- Clear objective and success criteria
- Specific deliverables expected
- Any constraints or requirements
- Context about the broader project

**Examples**:

**Worker Agent Example**:
```
Role: "Python Developer"
Task: "Create a web scraper that extracts product prices from Amazon search results for 'wireless headphones'. Save results to CSV with columns: name, price, rating, url. Handle pagination to get at least 100 products. Include error handling and respect rate limits."
Type: "Worker"
```

**Manager Agent Example**: 
```
Role: "DevOps Lead"  
Task: "Set up complete CI/CD pipeline for a Python Flask application. This includes: 1) GitHub Actions workflow, 2) Docker containerization, 3) AWS deployment configuration, 4) Database migration scripts, 5) Monitoring setup. Coordinate with team and delegate subtasks as needed."
Type: "Manager"
```

**SubManager Agent Example**:
```
Role: "Frontend Team Lead"
Task: "Develop the user interface for the e-commerce platform. Manage the implementation of: 1) Product listing pages, 2) Shopping cart functionality, 3) Checkout flow, 4) User account pages. Coordinate with the main project manager and delegate specific UI components to workers."
Type: "SubManager"
```

**Critical**: The more detailed your task description, the better results you'll get!"#;

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
        // Parse the tool parameters
        let params: SpawnAgentsInput =
            match serde_json::from_str(&tool_call.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    let error_msg = format!("Failed to parse spawn agent parameters: {}", e);
                    let error_result = ToolCallResult {
                        content: error_msg.clone(),
                        ui_display_info: UIDisplayInfo {
                            collapsed: "Parameters: Invalid format".to_string(),
                            expanded: Some(format!("Error: Failed to parse parameters\n\nDetails: {}", error_msg)),
                        },
                    };
                    self.send_error_result(&tool_call.tool_call.id, error_result);
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
            self.send_error_result(&tool_call.tool_call.id, error_result);
            return;
        }

        let mut spawned_agents = Vec::new();

        // Process each agent to spawn
        for agent_def in &params.agents_to_spawn {
            // Determine actors based on agent type
            let actors = match agent_def.agent_type {
                AgentType::Worker => self.config.worker_actors.clone(),
                AgentType::SubManager => self.config.sub_manager_actors.clone(),
                _ => unreachable!(),
            };

            // Use the host's spawn_agent function
            let agent_id =
                match bindings::hive::actor::agent::spawn_agent(&actors, &agent_def.agent_role) {
                    Ok(scope) => scope,
                    Err(e) => {
                        let error_result = ToolCallResult {
                            content: format!("Failed to spawn agent: {}", e),
                            ui_display_info: UIDisplayInfo {
                                collapsed: format!("Agent spawn: Failed to create {:?}", agent_def.agent_type),
                                expanded: Some(format!("Operation: Create agent\nAgent Type: {:?}\nError: {}", agent_def.agent_type, e)),
                            },
                        };
                        self.send_error_result(&tool_call.tool_call.id, error_result);
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
        }

        // If wait is true, send a status update request to make the agent wait
        if params.wait.unwrap_or(false) {
            let status_update_request = RequestStatusUpdate {
                agent: self.scope.clone(),
                status: Status::Wait {
                    reason: WaitReason::WaitingForAgentCoordination {
                        coordinating_tool_call_id: tool_call.tool_call.id.clone(),
                        coordinating_tool_name: "spawn_agent".to_string(),
                        target_agent_scope: None,
                        user_can_interrupt: true,
                    },
                },
                tool_call_id: Some(tool_call.tool_call.id.clone()),
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

        let result = ToolCallResult {
            content: success_message.clone(),
            ui_display_info: UIDisplayInfo {
                collapsed: format!(
                    "{} agent{} created: {}",
                    params.agents_to_spawn.len(),
                    if params.agents_to_spawn.len() == 1 { "" } else { "s" },
                    spawned_agents.join(", ")
                ),
                expanded: Some(format!("Operation: Create agents\n\n{}", success_message)),
            },
        };

        self.send_success_result(&tool_call.tool_call.id, result);
    }
}

impl SpawnAgentTool {
    fn send_error_result(&self, tool_call_id: &str, error_result: ToolCallResult) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(error_result),
            },
        };
        let _ = Self::broadcast_common_message(update);
    }

    fn send_success_result(&self, tool_call_id: &str, result: ToolCallResult) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done { result: Ok(result) },
        };
        let _ = Self::broadcast_common_message(update);
    }
}

// TODO: Improve the deserialization here action should be an enum we should user serde_json, etc...

use crate::llm_client::{Tool, ToolCall};
use tokio::sync::broadcast;

use crate::actors::{
    Actor, ActorContext, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, AgentType,
    InterAgentMessage, Message, ToolCallStatus, ToolCallUpdate, ToolDisplayInfo, WaitReason,
};
use crate::config::ParsedConfig;
use crate::scope::Scope;

// User-facing icons (for TUI)
const USER_STATUS_PENDING: &str = "[ ]";
const USER_STATUS_IN_PROGRESS: &str = "[~]";
const USER_STATUS_COMPLETED: &str = "[x]";
const USER_STATUS_SKIPPED: &str = "[>>]";

// Assistant-facing icons (for system prompts)
const ASSISTANT_STATUS_PENDING: &str = "[ ]";
const ASSISTANT_STATUS_IN_PROGRESS: &str = "[~]";
const ASSISTANT_STATUS_COMPLETED: &str = "[x]";
const ASSISTANT_STATUS_SKIPPED: &str = "[>>]";

pub fn format_planner_success_response_for_assistant(title: &str, agent_type: AgentType) -> String {
    format!(
        "Created task plan: {}{}",
        title,
        if agent_type == AgentType::Worker {
            " (awaiting manager approval)"
        } else {
            ""
        }
    )
}

pub fn format_request_plan_approval_message_for_assistant(plan: &TaskPlan) -> String {
    format!(
        "Before starting, please approve or reject my plan to acomplish my task: <plan>\n{}\n</plan>\n\nI am waiting for you to respond before proceeding.",
        plan.format_for_assistant()
    )
}

/// Task status for the planner
#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Skipped,
}

/// Individual task in the plan
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Task {
    pub description: String,
    pub status: TaskStatus,
}

impl Task {
    /// Get status icon for user display (TUI)
    pub fn user_status_icon(&self) -> &'static str {
        match self.status {
            TaskStatus::Pending => USER_STATUS_PENDING,
            TaskStatus::InProgress => USER_STATUS_IN_PROGRESS,
            TaskStatus::Completed => USER_STATUS_COMPLETED,
            TaskStatus::Skipped => USER_STATUS_SKIPPED,
        }
    }

    /// Get status icon for assistant display (system prompt)
    pub fn assistant_status_icon(&self) -> &'static str {
        match self.status {
            TaskStatus::Pending => ASSISTANT_STATUS_PENDING,
            TaskStatus::InProgress => ASSISTANT_STATUS_IN_PROGRESS,
            TaskStatus::Completed => ASSISTANT_STATUS_COMPLETED,
            TaskStatus::Skipped => ASSISTANT_STATUS_SKIPPED,
        }
    }
}

/// Task plan managed by the planner tool
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TaskPlan {
    pub title: String,
    pub tasks: Vec<Task>,
}

impl TaskPlan {
    /// Format the plan for TUI display without the "Plan:" prefix
    pub fn format_for_tui(&self) -> String {
        let mut result = String::new();
        for (i, task) in self.tasks.iter().enumerate() {
            result.push_str(&format!(
                "{}. {} {}\n",
                i + 1,
                task.user_status_icon(),
                task.description
            ));
        }
        result.trim_end().to_string()
    }

    /// Format the plan for assistant system prompt
    pub fn format_for_assistant(&self) -> String {
        let mut result = format!("Plan: {}\n", self.title);
        for (i, task) in self.tasks.iter().enumerate() {
            result.push_str(&format!(
                "{}. {} {}\n",
                i + 1,
                task.assistant_status_icon(),
                task.description
            ));
        }
        result.trim_end().to_string()
    }
}

/// Helper functions for creating TUI display information
fn create_collapsed_display(action: &str, context: &str) -> String {
    match action {
        "create" => format!("Plan created: {}", context),
        "update" => format!("Plan updated: {}", context),
        "complete" => format!("Task completed: {}", context),
        "start" => format!("Task started: {}", context),
        "skip" => format!("Task skipped: {}", context),
        _ => format!("Plan action: {}", action),
    }
}

fn create_expanded_display(action: &str, context: &str, plan: &TaskPlan) -> String {
    let action_message = create_collapsed_display(action, context);
    format!("{}\n{}", action_message, plan.format_for_tui())
}

fn create_tui_display_info(action: &str, context: &str, plan: &TaskPlan) -> ToolDisplayInfo {
    ToolDisplayInfo {
        collapsed: create_collapsed_display(action, context),
        expanded: Some(create_expanded_display(action, context, plan)),
    }
}

pub const TOOL_NAME: &str = "planner";
pub const TOOL_DESCRIPTION: &str = "Creates and manages a task plan with numbered steps. Actions: create (with title and tasks array), update (task_number and new_description), complete/start/skip (task_number)";
pub const TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "enum": ["create", "update", "complete", "start", "skip"],
            "description": "The action to perform on the task plan. If you already have an existing plan `create` will replace it."
        },
        "title": {
            "type": "string",
            "description": "Title of the task plan (required for 'create' action)"
        },
        "tasks": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Array of task descriptions (required for 'create' action)"
        },
        "task_number": {
            "type": "integer",
            "description": "The task number to update (1-based, required for update/complete/start/skip actions)"
        },
        "new_description": {
            "type": "string",
            "description": "New description for the task (required for 'update' action)"
        }
    },
    "required": ["action"]
}"#;

/// Planner actor
#[derive(hive_macros::ActorContext)]
pub struct Planner {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)] // TODO: Use for planning parameters, model settings
    config: ParsedConfig,
    current_task_plan: Option<TaskPlan>,
    scope: Scope,
    agent_type: AgentType,
    parent_scope: Option<Scope>,
}

impl Planner {
    pub fn new(
        config: ParsedConfig,
        tx: broadcast::Sender<ActorMessage>,
        scope: Scope,
        agent_type: AgentType,
        parent_scope: Option<Scope>,
    ) -> Self {
        Self {
            config,
            tx,
            current_task_plan: None,
            scope,
            agent_type,
            parent_scope,
        }
    }

    pub fn get_current_plan(&self) -> Option<&TaskPlan> {
        self.current_task_plan.as_ref()
    }

    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        if tool_call.function.name != TOOL_NAME {
            return;
        }

        // Parse the arguments
        let args = match serde_json::from_str::<serde_json::Value>(&tool_call.function.arguments) {
            Ok(args) => args,
            Err(e) => {
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.id,
                    status: ToolCallStatus::Finished {
                        result: Err(format!("Failed to parse planner arguments: {}", e)),
                        tui_display: Some(ToolDisplayInfo {
                            collapsed: "Error parsing planner arguments".to_string(),
                            expanded: None,
                        }),
                    },
                }));
                return;
            }
        };

        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(action) => action,
            None => {
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.id,
                    status: ToolCallStatus::Finished {
                        result: Err("Missing 'action' field".to_string()),
                        tui_display: Some(ToolDisplayInfo {
                            collapsed: "Error: Missing action field".to_string(),
                            expanded: None,
                        }),
                    },
                }));
                return;
            }
        };

        let _response_content = match action {
            "create" => self.handle_create_plan(&args, &tool_call.id).await,
            "update" | "complete" | "start" | "skip" => {
                self.handle_update_plan(action, &args, &tool_call.id).await
            }
            _ => {
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.id,
                    status: ToolCallStatus::Finished {
                        result: Err(format!("Unknown action: {}", action)),
                        tui_display: Some(ToolDisplayInfo {
                            collapsed: format!("Error: Unknown action '{}'", action),
                            expanded: None,
                        }),
                    },
                }));
                return;
            }
        };
    }

    async fn handle_create_plan(&mut self, args: &serde_json::Value, tool_call_id: &str) {
        let title = match args.get("title").and_then(|v| v.as_str()) {
            Some(title) => title,
            None => {
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call_id.to_string(),
                    status: ToolCallStatus::Finished {
                        result: Err("Missing 'title' field for create action".to_string()),
                        tui_display: Some(ToolDisplayInfo {
                            collapsed: "Error: Missing title field".to_string(),
                            expanded: None,
                        }),
                    },
                }));
                return;
            }
        };

        let tasks = match args.get("tasks").and_then(|v| v.as_array()) {
            Some(tasks) => tasks,
            None => {
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call_id.to_string(),
                    status: ToolCallStatus::Finished {
                        result: Err("Missing 'tasks' field for create action".to_string()),
                        tui_display: Some(ToolDisplayInfo {
                            collapsed: "Error: Missing tasks field".to_string(),
                            expanded: None,
                        }),
                    },
                }));
                return;
            }
        };

        let mut task_list = Vec::new();
        for task in tasks {
            if let Some(desc) = task.as_str() {
                task_list.push(Task {
                    description: desc.to_string(),
                    status: TaskStatus::Pending,
                });
            }
        }

        if task_list.is_empty() {
            self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call_id.to_string(),
                status: ToolCallStatus::Finished {
                    result: Err("Task list cannot be empty".to_string()),
                    tui_display: Some(ToolDisplayInfo {
                        collapsed: "Error: Task list cannot be empty".to_string(),
                        expanded: None,
                    }),
                },
            }));
            return;
        }

        let plan = TaskPlan {
            title: title.to_string(),
            tasks: task_list,
        };

        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status: ToolCallStatus::Received,
        }));

        // Store the plan
        self.current_task_plan = Some(plan.clone());

        // Send system state update
        self.broadcast(Message::PlanUpdated(plan.clone()));

        // For Worker agents, broadcast AwaitingManager status to request plan approval
        if self.agent_type == AgentType::Worker {
            self.broadcast(Message::Agent(AgentMessage {
                agent_id: self.scope.clone(),
                message: AgentMessageType::InterAgentMessage(
                    InterAgentMessage::StatusUpdateRequest {
                        tool_call_id: tool_call_id.to_string(),
                        status: AgentStatus::Wait {
                            reason: WaitReason::WaitingForManager {
                                tool_name: Some(TOOL_NAME.to_string()),
                                tool_call_id: tool_call_id.to_owned(),
                            },
                        },
                    },
                ),
            }));

            self.broadcast(Message::Agent(AgentMessage {
                agent_id: self.parent_scope.clone().unwrap(),
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::Message {
                    message: format_request_plan_approval_message_for_assistant(&plan),
                }),
            }));
        }

        // Return concise response
        let response = format_planner_success_response_for_assistant(&title, self.agent_type);

        // Create TUI display info
        let tui_display = Some(create_tui_display_info("create", &plan.title, &plan));

        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status: ToolCallStatus::Finished {
                result: Ok(response),
                tui_display,
            },
        }));
    }

    async fn handle_update_plan(
        &mut self,
        action: &str,
        args: &serde_json::Value,
        tool_call_id: &str,
    ) {
        let mut task_plan = match self.current_task_plan.clone() {
            Some(plan) => plan,
            None => {
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call_id.to_string(),
                    status: ToolCallStatus::Finished {
                        result: Err("No active task plan. Create a plan first.".to_string()),
                        tui_display: Some(ToolDisplayInfo {
                            collapsed: "Error: No active task plan".to_string(),
                            expanded: None,
                        }),
                    },
                }));
                return;
            }
        };

        let task_number = match args.get("task_number").and_then(|v| v.as_u64()) {
            Some(num) => num as usize,
            None => {
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call_id.to_string(),
                    status: ToolCallStatus::Finished {
                        result: Err("Missing 'task_number' field".to_string()),
                        tui_display: Some(ToolDisplayInfo {
                            collapsed: "Error: Missing task number".to_string(),
                            expanded: None,
                        }),
                    },
                }));
                return;
            }
        };

        if task_number == 0 || task_number > task_plan.tasks.len() {
            self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call_id.to_string(),
                status: ToolCallStatus::Finished {
                    result: Err(format!(
                        "Invalid task number. Must be between 1 and {}",
                        task_plan.tasks.len()
                    )),
                    tui_display: Some(ToolDisplayInfo {
                        collapsed: "Error: Invalid task number".to_string(),
                        expanded: None,
                    }),
                },
            }));
            return;
        }

        let task_index = task_number - 1;

        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status: ToolCallStatus::Received,
        }));

        // Update the task plan
        match action {
            "update" => {
                if let Some(new_desc) = args.get("new_description").and_then(|v| v.as_str()) {
                    task_plan.tasks[task_index].description = new_desc.to_string();
                }
            }
            "complete" => {
                task_plan.tasks[task_index].status = TaskStatus::Completed;
            }
            "start" => {
                task_plan.tasks[task_index].status = TaskStatus::InProgress;
            }
            "skip" => {
                task_plan.tasks[task_index].status = TaskStatus::Skipped;
            }
            _ => unreachable!(),
        }
        self.current_task_plan = Some(task_plan.clone());

        // Send system state update
        self.broadcast(Message::PlanUpdated(task_plan.clone()));

        // Return concise response
        let response = match action {
            "update" => format!("Updated task {}", task_number),
            "complete" => format!("Completed task {}", task_number),
            "start" => format!("Started task {}", task_number),
            "skip" => format!("Skipped task {}", task_number),
            _ => unreachable!(),
        };

        // Create TUI display info
        let context = format!("task {}", task_number);
        let tui_display = Some(create_tui_display_info(action, &context, &task_plan));

        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status: ToolCallStatus::Finished {
                result: Ok(response),
                tui_display,
            },
        }));
    }
}

#[async_trait::async_trait]
impl Actor for Planner {
    const ACTOR_ID: &'static str = "planner";

    async fn on_start(&mut self) {
        let tool = Tool {
            tool_type: "function".to_string(),
            function: crate::llm_client::ToolFunction {
                name: TOOL_NAME.to_string(),
                description: TOOL_DESCRIPTION.to_string(),
                parameters: serde_json::from_str(TOOL_INPUT_SCHEMA).unwrap(),
            },
        };

        self.broadcast(Message::ToolsAvailable(vec![tool]));
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        match message.message {
            Message::AssistantToolCall(tool_call) => self.handle_tool_call(tool_call).await,
            _ => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_for_tui() {
        let plan = TaskPlan {
            title: "Test Plan".to_string(),
            tasks: vec![
                Task {
                    description: "Pending task".to_string(),
                    status: TaskStatus::Pending,
                },
                Task {
                    description: "In progress task".to_string(),
                    status: TaskStatus::InProgress,
                },
                Task {
                    description: "Completed task".to_string(),
                    status: TaskStatus::Completed,
                },
                Task {
                    description: "Skipped task".to_string(),
                    status: TaskStatus::Skipped,
                },
            ],
        };

        let expected = "1. [ ] Pending task\n2. [~] In progress task\n3. [x] Completed task\n4. [>>] Skipped task";
        assert_eq!(plan.format_for_tui(), expected);
    }

    #[test]
    fn test_format_for_assistant() {
        let plan = TaskPlan {
            title: "Test Plan".to_string(),
            tasks: vec![
                Task {
                    description: "Pending task".to_string(),
                    status: TaskStatus::Pending,
                },
                Task {
                    description: "In progress task".to_string(),
                    status: TaskStatus::InProgress,
                },
                Task {
                    description: "Completed task".to_string(),
                    status: TaskStatus::Completed,
                },
                Task {
                    description: "Skipped task".to_string(),
                    status: TaskStatus::Skipped,
                },
            ],
        };

        let expected = "Plan: Test Plan\n1. [ ] Pending task\n2. [~] In progress task\n3. [x] Completed task\n4. [>>] Skipped task";
        assert_eq!(plan.format_for_assistant(), expected);
    }
}

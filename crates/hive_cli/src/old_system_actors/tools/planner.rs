use crate::llm_client::ToolCall;
use tokio::sync::broadcast;

use crate::actors::{
    ActorContext, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, AgentType,
    InterAgentMessage, Message, ToolCallResult, ToolDisplayInfo, WaitReason,
};
use crate::config::ParsedConfig;
use crate::scope::Scope;

use super::Tool;

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

#[derive(Debug, serde::Deserialize)]
pub enum PlannerAction {
    #[serde(rename = "create")]
    Create,
    #[serde(rename = "update")]
    Update,
    #[serde(rename = "complete")]
    Complete,
    #[serde(rename = "start")]
    Start,
    #[serde(rename = "skip")]
    Skip,
}

#[derive(Debug, serde::Deserialize)]
pub struct PlannerParams {
    action: PlannerAction,
    title: Option<String>,
    tasks: Option<Vec<String>>,
    task_number: Option<u64>,
    new_description: Option<String>,
}

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

    async fn handle_create_plan(&mut self, params: &PlannerParams, tool_call_id: &str) {
        let title = match &params.title {
            Some(title) => title,
            None => {
                self.broadcast_finished(
                    tool_call_id,
                    ToolCallResult::Err("Missing 'title' field for create action".to_string()),
                    Some(ToolDisplayInfo {
                        collapsed: "Error: Missing title field".to_string(),
                        expanded: None,
                    }),
                );
                return;
            }
        };

        let tasks = match &params.tasks {
            Some(tasks) => tasks,
            None => {
                self.broadcast_finished(
                    tool_call_id,
                    ToolCallResult::Err("Missing 'tasks' field for create action".to_string()),
                    Some(ToolDisplayInfo {
                        collapsed: "Error: Missing tasks field".to_string(),
                        expanded: None,
                    }),
                );
                return;
            }
        };

        let mut task_list = Vec::new();
        for task in tasks {
            task_list.push(Task {
                description: task.clone(),
                status: TaskStatus::Pending,
            });
        }

        if task_list.is_empty() {
            self.broadcast_finished(
                tool_call_id,
                ToolCallResult::Err("Task list cannot be empty".to_string()),
                Some(ToolDisplayInfo {
                    collapsed: "Error: Task list cannot be empty".to_string(),
                    expanded: None,
                }),
            );
            return;
        }

        let plan = TaskPlan {
            title: title.to_string(),
            tasks: task_list,
        };

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
                                tool_name: Some(Planner::TOOL_NAME.to_string()),
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

        self.broadcast_finished(tool_call_id, ToolCallResult::Ok(response), tui_display);
    }

    async fn handle_update_plan(
        &mut self,
        action: &str,
        params: &PlannerParams,
        tool_call_id: &str,
    ) {
        let mut task_plan = match self.current_task_plan.clone() {
            Some(plan) => plan,
            None => {
                self.broadcast_finished(
                    tool_call_id,
                    ToolCallResult::Err("No active task plan. Create a plan first.".to_string()),
                    Some(ToolDisplayInfo {
                        collapsed: "Error: No active task plan".to_string(),
                        expanded: None,
                    }),
                );
                return;
            }
        };

        let task_number = match params.task_number {
            Some(num) => num as usize,
            None => {
                self.broadcast_finished(
                    tool_call_id,
                    ToolCallResult::Err("Missing 'task_number' field".to_string()),
                    Some(ToolDisplayInfo {
                        collapsed: "Error: Missing task number".to_string(),
                        expanded: None,
                    }),
                );
                return;
            }
        };

        if task_number == 0 || task_number > task_plan.tasks.len() {
            self.broadcast_finished(
                tool_call_id,
                ToolCallResult::Err(format!(
                    "Invalid task number. Must be between 1 and {}",
                    task_plan.tasks.len()
                )),
                Some(ToolDisplayInfo {
                    collapsed: "Error: Invalid task number".to_string(),
                    expanded: None,
                }),
            );
            return;
        }

        let task_index = task_number - 1;

        // Update the task plan
        match action {
            "update" => {
                if let Some(new_desc) = &params.new_description {
                    task_plan.tasks[task_index].description = new_desc.clone();
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

        self.broadcast_finished(tool_call_id, ToolCallResult::Ok(response), tui_display);
    }
}

#[async_trait::async_trait]
impl Tool for Planner {
    const TOOL_NAME: &str = "planner";
    const TOOL_DESCRIPTION: &str = "Creates and manages a task plan with numbered steps. Actions: create (with title and tasks array), update (task_number and new_description), complete/start/skip (task_number)";
    const TOOL_INPUT_SCHEMA: &str = r#"{
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

    type Params = PlannerParams;

    async fn execute_tool_call(&mut self, tool_call: ToolCall, params: Self::Params) {
        match params.action {
            PlannerAction::Create => self.handle_create_plan(&params, &tool_call.id).await,
            PlannerAction::Update => {
                self.handle_update_plan("update", &params, &tool_call.id)
                    .await
            }
            PlannerAction::Complete => {
                self.handle_update_plan("complete", &params, &tool_call.id)
                    .await
            }
            PlannerAction::Start => {
                self.handle_update_plan("start", &params, &tool_call.id)
                    .await
            }
            PlannerAction::Skip => {
                self.handle_update_plan("skip", &params, &tool_call.id)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    fn create_test_planner() -> Planner {
        let (tx, _) = broadcast::channel(100);
        let config = crate::config::Config::new(true)
            .unwrap()
            .try_into()
            .unwrap();
        let scope = Scope::new();
        Planner::new(config, tx, scope, crate::actors::AgentType::Worker, None)
    }

    #[test]
    fn test_planner_deserialize_params_success() {
        let planner = create_test_planner();
        let json_input = r#"{
            "action": "create",
            "title": "Test Plan",
            "tasks": ["Task 1", "Task 2"]
        }"#;

        let result = planner.deserialize_params(json_input);
        assert!(result.is_ok());

        let params = result.unwrap();
        assert!(matches!(params.action, PlannerAction::Create));
        assert_eq!(params.title, Some("Test Plan".to_string()));
        assert_eq!(
            params.tasks,
            Some(vec!["Task 1".to_string(), "Task 2".to_string()])
        );
    }

    #[test]
    fn test_planner_deserialize_params_failure() {
        let planner = create_test_planner();
        let json_input = r#"{"action": "invalid_action"}"#;

        let result = planner.deserialize_params(json_input);
        assert!(result.is_err());
    }

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

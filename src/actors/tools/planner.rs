use genai::chat::{Tool, ToolCall};
use std::fmt;
use tokio::sync::broadcast;
use tracing::info;
use uuid::Uuid;

use crate::actors::{
    Actor, ActorMessage, AgentMessage, AgentMessageType, AgentTaskStatus, AgentType,
    InterAgentMessage, Message, TaskAwaitingManager, ToolCallStatus, ToolCallType, ToolCallUpdate,
};
use crate::config::ParsedConfig;

/// Task status for the planner
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Skipped,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let icon = match self {
            TaskStatus::Pending => "[ ]",
            TaskStatus::InProgress => "[~]",
            TaskStatus::Completed => "[x]",
            TaskStatus::Skipped => "[>>]",
        };
        write!(f, "{}", icon)
    }
}

/// Individual task in the plan
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Task {
    pub description: String,
    pub status: TaskStatus,
}

impl Task {
    /// Get status icon
    pub fn status_icon(&self) -> &'static str {
        match self.status {
            TaskStatus::Pending => "[ ]",
            TaskStatus::InProgress => "[~]",
            TaskStatus::Completed => "[x]",
            TaskStatus::Skipped => "[>>]",
        }
    }
}

/// Task plan managed by the planner tool
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskPlan {
    pub title: String,
    pub tasks: Vec<Task>,
}

impl fmt::Display for TaskPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "## Current Task Plan: {}", self.title)?;
        for (i, task) in self.tasks.iter().enumerate() {
            writeln!(f, "{}. {} {}", i + 1, task.status, task.description)?;
        }
        Ok(())
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
            "description": "The action to perform on the task plan"
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
pub struct Planner {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)] // TODO: Use for planning parameters, model settings
    config: ParsedConfig,
    current_task_plan: Option<TaskPlan>,
    scope: Uuid,
    agent_type: AgentType,
}

impl Planner {
    pub fn new(
        config: ParsedConfig,
        tx: broadcast::Sender<ActorMessage>,
        scope: Uuid,
        agent_type: AgentType,
    ) -> Self {
        Self {
            config,
            tx,
            current_task_plan: None,
            scope,
            agent_type,
        }
    }

    pub fn get_current_plan(&self) -> Option<&TaskPlan> {
        self.current_task_plan.as_ref()
    }

    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        if tool_call.fn_name != TOOL_NAME {
            return;
        }

        // Parse the arguments
        let args = match serde_json::from_value::<serde_json::Value>(tool_call.fn_arguments) {
            Ok(args) => args,
            Err(e) => {
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!(
                        "Failed to parse planner arguments: {}",
                        e
                    ))),
                }));
                return;
            }
        };

        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(action) => action,
            None => {
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err("Missing 'action' field".to_string())),
                }));
                return;
            }
        };

        let _response_content = match action {
            "create" => self.handle_create_plan(&args, &tool_call.call_id).await,
            "update" | "complete" | "start" | "skip" => {
                self.handle_update_plan(action, &args, &tool_call.call_id)
                    .await
            }
            _ => {
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!("Unknown action: {}", action))),
                }));
                return;
            }
        };
    }

    async fn handle_create_plan(&mut self, args: &serde_json::Value, tool_call_id: &str) {
        let title = match args.get("title").and_then(|v| v.as_str()) {
            Some(title) => title,
            None => {
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call_id.to_string(),
                    status: ToolCallStatus::Finished(Err(
                        "Missing 'title' field for create action".to_string(),
                    )),
                }));
                return;
            }
        };

        let tasks = match args.get("tasks").and_then(|v| v.as_array()) {
            Some(tasks) => tasks,
            None => {
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call_id.to_string(),
                    status: ToolCallStatus::Finished(Err(
                        "Missing 'tasks' field for create action".to_string(),
                    )),
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
            let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call_id.to_string(),
                status: ToolCallStatus::Finished(Err("Task list cannot be empty".to_string())),
            }));
            return;
        }

        let plan = TaskPlan {
            title: title.to_string(),
            tasks: task_list,
        };

        let friendly_command_display = format!("Create task plan: {}", title);
        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::Planner,
                friendly_command_display,
            },
        }));

        // Store the plan
        self.current_task_plan = Some(plan.clone());

        // Send system state update
        let _ = self.broadcast(Message::PlanUpdated(plan.clone()));

        // For Worker agents, broadcast AwaitingManager status to request plan approval
        if self.agent_type == AgentType::Worker {
            let _ = self.broadcast(Message::Agent(AgentMessage {
                agent_id: self.scope.clone(),
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status: AgentTaskStatus::AwaitingManager(
                        TaskAwaitingManager::AwaitingPlanApproval(plan.clone()),
                    ),
                }),
            }));
        }

        // Return concise response
        let response = format!(
            "Created task plan: {} with {} tasks{}",
            title,
            plan.tasks.len(),
            if self.agent_type == AgentType::Worker {
                " (awaiting manager approval)"
            } else {
                ""
            }
        );

        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status: ToolCallStatus::Finished(Ok(response)),
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
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call_id.to_string(),
                    status: ToolCallStatus::Finished(Err(
                        "No active task plan. Create a plan first.".to_string(),
                    )),
                }));
                return;
            }
        };

        let task_number = match args.get("task_number").and_then(|v| v.as_u64()) {
            Some(num) => num as usize,
            None => {
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call_id.to_string(),
                    status: ToolCallStatus::Finished(
                        Err("Missing 'task_number' field".to_string()),
                    ),
                }));
                return;
            }
        };

        if task_number == 0 || task_number > task_plan.tasks.len() {
            let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call_id.to_string(),
                status: ToolCallStatus::Finished(Err(format!(
                    "Invalid task number. Must be between 1 and {}",
                    task_plan.tasks.len()
                ))),
            }));
            return;
        }

        let task_index = task_number - 1;

        let friendly_command_display = match action {
            "update" => format!("Update task {} in plan", task_number),
            "complete" => format!("Complete task {} in plan", task_number),
            "start" => format!("Start task {} in plan", task_number),
            "skip" => format!("Skip task {} in plan", task_number),
            _ => unreachable!(),
        };

        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::Planner,
                friendly_command_display,
            },
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
        let _ = self.broadcast(Message::PlanUpdated(task_plan.clone()));

        // Return concise response
        let response = match action {
            "update" => format!("Updated task {}", task_number),
            "complete" => format!("Completed task {}", task_number),
            "start" => format!("Started task {}", task_number),
            "skip" => format!("Skipped task {}", task_number),
            _ => unreachable!(),
        };
        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status: ToolCallStatus::Finished(Ok(response)),
        }));
    }
}

#[async_trait::async_trait]
impl Actor for Planner {
    const ACTOR_ID: &'static str = "planner";

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    fn get_scope(&self) -> &Uuid {
        &self.scope
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    async fn on_start(&mut self) {
        let tool = Tool {
            name: TOOL_NAME.to_string(),
            description: Some(TOOL_DESCRIPTION.to_string()),
            schema: Some(serde_json::from_str(TOOL_INPUT_SCHEMA).unwrap()),
        };

        let _ = self.broadcast(Message::ToolsAvailable(vec![tool]));
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        match message.message {
            Message::AssistantToolCall(tool_call) => self.handle_tool_call(tool_call).await,
            _ => (),
        }
    }
}

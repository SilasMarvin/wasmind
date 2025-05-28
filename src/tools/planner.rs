use serde_json;
use genai::chat::ToolResponse;
use crossbeam::channel::Sender;
use tracing::error;
use serde_json::{json, Value};

use crate::{worker::Event, tui};

/// Task status for the planner
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Skipped,
}

/// Individual task in the plan
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Task {
    pub description: String,
    pub status: TaskStatus,
}

/// Task plan managed by the planner tool
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskPlan {
    pub title: String,
    pub tasks: Vec<Task>,
}

pub struct Planner {}

impl Planner {
    pub fn new() -> Self {
        Planner {}
    }
}

impl crate::tools::InternalTool for Planner {
    fn name(&self) -> &'static str {
        "planner"
    }
    
    fn description(&self) -> &'static str {
        "Creates and manages a task plan with numbered steps. Actions: create (with title and tasks array), update (task_number and new_description), complete/start/skip (task_number)"
    }
    
    fn input_schema(&self) -> Value {
        json!({
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
        })
    }
}

/// Handle the planner tool call (legacy function for worker.rs)
pub fn handle_planner(
    tool_call: genai::chat::ToolCall,
    current_task_plan: &mut Option<TaskPlan>,
    tui_tx: &Sender<tui::Task>,
    worker_tx: &Sender<Event>,
) {
    // Parse the arguments
    let args = match serde_json::from_value::<serde_json::Value>(tool_call.fn_arguments) {
        Ok(args) => args,
        Err(e) => {
            error!("Failed to parse planner arguments: {}", e);
            send_planner_error(worker_tx, tool_call.call_id, "Failed to parse arguments");
            return;
        }
    };
    
    let action = match args.get("action").and_then(|v| v.as_str()) {
        Some(action) => action,
        None => {
            error!("Missing 'action' field in planner arguments");
            send_planner_error(worker_tx, tool_call.call_id, "Missing 'action' field");
            return;
        }
    };
    
    match action {
        "create" => handle_create_plan(&args, current_task_plan, tui_tx, worker_tx, tool_call.call_id),
        "update" | "complete" | "start" | "skip" => {
            handle_update_plan(action, &args, current_task_plan, tui_tx, worker_tx, tool_call.call_id)
        }
        _ => {
            send_planner_error(worker_tx, tool_call.call_id, &format!("Unknown action: {}", action));
        }
    }
}

fn handle_create_plan(
    args: &serde_json::Value,
    current_task_plan: &mut Option<TaskPlan>,
    tui_tx: &Sender<tui::Task>,
    worker_tx: &Sender<Event>,
    call_id: String,
) {
    let title = match args.get("title").and_then(|v| v.as_str()) {
        Some(title) => title,
        None => {
            send_planner_error(worker_tx, call_id, "Missing 'title' field for create action");
            return;
        }
    };
    
    let tasks = match args.get("tasks").and_then(|v| v.as_array()) {
        Some(tasks) => {
            let mut task_list = Vec::new();
            for task in tasks {
                if let Some(desc) = task.as_str() {
                    task_list.push(Task {
                        description: desc.to_string(),
                        status: TaskStatus::Pending,
                    });
                }
            }
            task_list
        }
        None => {
            send_planner_error(worker_tx, call_id, "Missing 'tasks' field for create action");
            return;
        }
    };
    
    if tasks.is_empty() {
        send_planner_error(worker_tx, call_id, "Task list cannot be empty");
        return;
    }
    
    let plan = TaskPlan {
        title: title.to_string(),
        tasks,
    };
    
    // Send TUI event to display the plan
    let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::task_plan_created(
        plan.clone()
    )));
    
    // Store the plan
    *current_task_plan = Some(plan.clone());
    
    // Send success response with numbered tasks
    let mut response = format!("Created task plan: {}\n", title);
    for (i, task) in plan.tasks.iter().enumerate() {
        response.push_str(&format!("{}. {}\n", i + 1, task.description));
    }
    
    send_planner_response(worker_tx, call_id, response);
}

fn handle_update_plan(
    action: &str,
    args: &serde_json::Value,
    current_task_plan: &mut Option<TaskPlan>,
    tui_tx: &Sender<tui::Task>,
    worker_tx: &Sender<Event>,
    call_id: String,
) {
    let task_plan = match current_task_plan.as_mut() {
        Some(plan) => plan,
        None => {
            send_planner_error(worker_tx, call_id, "No active task plan. Create a plan first.");
            return;
        }
    };
    
    let task_number = match args.get("task_number").and_then(|v| v.as_u64()) {
        Some(num) => num as usize,
        None => {
            send_planner_error(worker_tx, call_id, "Missing 'task_number' field");
            return;
        }
    };
    
    if task_number == 0 || task_number > task_plan.tasks.len() {
        send_planner_error(worker_tx, call_id, 
            &format!("Invalid task number. Must be between 1 and {}", task_plan.tasks.len()));
        return;
    }
    
    let task_index = task_number - 1;
    
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
    
    // Send TUI event to update the display
    let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::task_plan_updated(
        task_plan.clone()
    )));
    
    // Send success response
    let response = format!("Updated task {}: {}", task_number, task_plan.tasks[task_index].description);
    send_planner_response(worker_tx, call_id, response);
}

fn send_planner_response(worker_tx: &Sender<Event>, call_id: String, content: String) {
    let tool_response = ToolResponse {
        call_id,
        content,
    };
    let _ = worker_tx.send(Event::MCPToolsResponse(vec![tool_response]));
}

fn send_planner_error(worker_tx: &Sender<Event>, call_id: String, error: &str) {
    let tool_response = ToolResponse {
        call_id,
        content: format!("Error: {}", error),
    };
    let _ = worker_tx.send(Event::MCPToolsResponse(vec![tool_response]));
}
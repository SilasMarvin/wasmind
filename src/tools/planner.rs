use genai::chat::{ToolCall, ToolResponse};
use crossbeam::channel::Sender;
use serde_json::Value;
use std::fmt;

use crate::tui;

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


pub struct Planner {
    current_task_plan: Option<TaskPlan>,
}

impl Planner {
    pub fn new() -> Self {
        Planner {
            current_task_plan: None,
        }
    }
    
    pub fn get_current_plan(&self) -> Option<&TaskPlan> {
        self.current_task_plan.as_ref()
    }
    
    pub fn name(&self) -> &'static str {
        TOOL_NAME
    }
    
    pub fn description(&self) -> &'static str {
        TOOL_DESCRIPTION
    }
    
    pub fn input_schema(&self) -> Value {
        serde_json::from_str(TOOL_INPUT_SCHEMA).unwrap()
    }
    
    pub fn handle_call(
        &mut self,
        tool_call: ToolCall,
        tui_tx: &Sender<tui::Task>,
    ) -> Result<Option<ToolResponse>, String> {
        // Parse the arguments
        let args = serde_json::from_value::<serde_json::Value>(tool_call.fn_arguments)
            .map_err(|e| format!("Failed to parse planner arguments: {}", e))?;
        
        let action = args.get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing 'action' field".to_string())?;
        
        let response_content = match action {
            "create" => self.handle_create_plan(&args, tui_tx)?,
            "update" | "complete" | "start" | "skip" => {
                self.handle_update_plan(action, &args, tui_tx)?
            }
            _ => return Err(format!("Unknown action: {}", action)),
        };
        
        Ok(Some(ToolResponse {
            call_id: tool_call.call_id,
            content: response_content,
        }))
    }
    
    fn handle_create_plan(
        &mut self,
        args: &serde_json::Value,
        tui_tx: &Sender<tui::Task>,
    ) -> Result<String, String> {
        let title = args.get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing 'title' field for create action".to_string())?;
        
        let tasks = args.get("tasks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "Missing 'tasks' field for create action".to_string())?;
        
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
            return Err("Task list cannot be empty".to_string());
        }
        
        let plan = TaskPlan {
            title: title.to_string(),
            tasks: task_list,
        };
        
        // Send TUI event to display the plan
        let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::task_plan_created(
            plan.clone()
        )));
        
        // Store the plan
        self.current_task_plan = Some(plan.clone());
        
        // Return response with numbered tasks
        let mut response = format!("Created task plan: {}\n", title);
        for (i, task) in plan.tasks.iter().enumerate() {
            response.push_str(&format!("{}. {}\n", i + 1, task.description));
        }
        
        Ok(response)
    }
    
    fn handle_update_plan(
        &mut self,
        action: &str,
        args: &serde_json::Value,
        tui_tx: &Sender<tui::Task>,
    ) -> Result<String, String> {
        let task_plan = self.current_task_plan.as_mut()
            .ok_or_else(|| "No active task plan. Create a plan first.".to_string())?;
        
        let task_number = args.get("task_number")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "Missing 'task_number' field".to_string())? as usize;
        
        if task_number == 0 || task_number > task_plan.tasks.len() {
            return Err(format!("Invalid task number. Must be between 1 and {}", task_plan.tasks.len()));
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
        
        // Return response
        Ok(format!("Updated task {}: {}", task_number, task_plan.tasks[task_index].description))
    }
}
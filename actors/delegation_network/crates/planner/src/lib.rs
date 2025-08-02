use hive_actor_utils::{
    common_messages::{
        assistant::{Section, SystemPromptContent, SystemPromptContribution},
        tools::{ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, UIDisplayInfo},
    },
    messages::Message,
    tools,
};

#[allow(warnings)]
mod bindings;

const PLANNER_USAGE_GUIDE: &str = r#"## planner Tool - Strategic Planning and Progress Tracking

**Purpose**: Create structured plans for complex multi-step tasks and track progress systematically.

**When to Use**:
- ✅ Complex projects with multiple sequential phases
- ✅ Tasks requiring coordination of multiple parallel efforts  
- ✅ Need to track progress across various components
- ✅ Want to break down large objectives into manageable steps
- ✅ Need formal planning before executing complex workflows

**When to Skip**:
- ❌ Simple, single-step tasks
- ❌ When you already have a clear mental plan
- ❌ Urgent tasks that need immediate action

**Planning Best Practices**:
- Start with clear success criteria
- Break work into logical phases
- Identify dependencies between tasks
- Consider resource requirements
- Plan for potential risks/blockers
- Include validation/testing phases

**Example Plans**:

📊 **Data Analysis Project**:
```
Title: "Customer Behavior Analysis Dashboard"
Steps:
1. Data Collection - Extract customer data from databases
2. Data Cleaning - Handle missing values, normalize formats  
3. Analysis - Run statistical analysis and generate insights
4. Visualization - Create interactive dashboard
5. Validation - Review results with stakeholders
6. Deployment - Set up automated reporting
```

🔧 **Software Development**:
```
Title: "E-commerce Payment Integration" 
Steps:
1. Requirements Analysis - Define payment provider specs
2. API Integration - Implement payment gateway connections
3. Database Schema - Design transaction storage
4. Testing - Unit tests, integration tests, security tests
5. Documentation - API docs and user guides
6. Deployment - Production rollout with monitoring
```

**Progress Tracking**: Use this tool to update plan status as work progresses and identify any blockers."#;

#[derive(Debug, serde::Deserialize)]
struct Task {
    description: String,
    status: TaskStatus,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Skipped,
}

impl TaskStatus {
    fn icon(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "[ ]",
            TaskStatus::InProgress => "[~]",
            TaskStatus::Completed => "[x]",
            TaskStatus::Skipped => "[>>]",
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct TaskPlan {
    title: String,
    tasks: Vec<Task>,
}

impl TaskPlan {
    fn format_for_display(&self) -> String {
        let mut result = format!("Plan: {}\n", self.title);
        for (i, task) in self.tasks.iter().enumerate() {
            result.push_str(&format!(
                "{}. {} {}\n",
                i + 1,
                task.status.icon(),
                task.description
            ));
        }
        result.trim_end().to_string()
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "action")]
#[serde(rename_all = "lowercase")]
enum PlannerAction {
    Create {
        title: String,
        tasks: Vec<String>,
    },
    Update {
        task_number: usize,
        status: TaskStatus,
    },
}

#[derive(tools::macros::Tool)]
#[tool(
    name = "planner",
    description = "Creates and manages a task plan with numbered steps. Use this to break down complex work into manageable tasks and track progress.",
    schema = r#"{
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["create", "update"],
                "description": "The action to perform: create a new plan or update an existing task"
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
                "description": "The task number to update (1-based, required for 'update' action)"
            },
            "status": {
                "type": "string",
                "enum": ["pending", "inprogress", "completed", "skipped"],
                "description": "New status for the task (required for 'update' action)"
            }
        },
        "required": ["action"]
    }"#
)]
struct PlannerTool {
    scope: String,
    current_plan: Option<TaskPlan>,
}

impl tools::Tool for PlannerTool {
    fn new(scope: String, _config: String) -> Self {
        // Broadcast guidance about how to use the planner tool
        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "planner:usage_guide".to_string(),
            content: SystemPromptContent::Text(PLANNER_USAGE_GUIDE.to_string()),
            priority: 800,
            section: Some(Section::Tools),
        });

        Self {
            scope,
            current_plan: None,
        }
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        // Parse the tool parameters
        let action: PlannerAction = match serde_json::from_str(&tool_call.tool_call.function.arguments) {
            Ok(action) => action,
            Err(e) => {
                let error_msg = format!("Failed to parse planner parameters: {}", e);
                let error_result = ToolCallResult {
                    content: error_msg.clone(),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "❌ Parameter Error".to_string(),
                        expanded: Some(format!("❌ Parameter Error:\n{}", error_msg)),
                    },
                };
                self.send_error_result(&tool_call.tool_call.id, error_result);
                return;
            }
        };

        match action {
            PlannerAction::Create { title, tasks } => {
                self.handle_create_plan(title, tasks, &tool_call.tool_call.id);
            }
            PlannerAction::Update { task_number, status } => {
                self.handle_update_task(task_number, status, &tool_call.tool_call.id);
            }
        }
    }
}

impl PlannerTool {
    fn handle_create_plan(&mut self, title: String, task_descriptions: Vec<String>, tool_call_id: &str) {
        if task_descriptions.is_empty() {
            let error_result = ToolCallResult {
                content: "Task list cannot be empty".to_string(),
                ui_display_info: UIDisplayInfo {
                    collapsed: "❌ Empty task list".to_string(),
                    expanded: Some("Task list cannot be empty".to_string()),
                },
            };
            self.send_error_result(tool_call_id, error_result);
            return;
        }

        let tasks: Vec<Task> = task_descriptions
            .into_iter()
            .map(|desc| Task {
                description: desc,
                status: TaskStatus::Pending,
            })
            .collect();

        let plan = TaskPlan { title: title.clone(), tasks };

        let plan_display = plan.format_for_display();
        self.current_plan = Some(plan);

        let result = ToolCallResult {
            content: format!("Created task plan: {}", title),
            ui_display_info: UIDisplayInfo {
                collapsed: format!("📋 Plan created: {}", title),
                expanded: Some(format!("📋 Plan created: {}\n\n{}", title, plan_display)),
            },
        };

        self.send_success_result(tool_call_id, result);
    }

    fn handle_update_task(&mut self, task_number: usize, new_status: TaskStatus, tool_call_id: &str) {
        let plan = match &mut self.current_plan {
            Some(plan) => plan,
            None => {
                let error_result = ToolCallResult {
                    content: "No active task plan. Create a plan first.".to_string(),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "❌ No active plan".to_string(),
                        expanded: Some("No active task plan. Create a plan first.".to_string()),
                    },
                };
                self.send_error_result(tool_call_id, error_result);
                return;
            }
        };

        if task_number == 0 || task_number > plan.tasks.len() {
            let error_result = ToolCallResult {
                content: format!(
                    "Invalid task number. Must be between 1 and {}",
                    plan.tasks.len()
                ),
                ui_display_info: UIDisplayInfo {
                    collapsed: "❌ Invalid task number".to_string(),
                    expanded: Some(format!(
                        "Invalid task number {}. Must be between 1 and {}",
                        task_number,
                        plan.tasks.len()
                    )),
                },
            };
            self.send_error_result(tool_call_id, error_result);
            return;
        }

        let task_index = task_number - 1;
        plan.tasks[task_index].status = new_status;

        let action_text = match &plan.tasks[task_index].status {
            TaskStatus::Pending => "reset to pending",
            TaskStatus::InProgress => "started",
            TaskStatus::Completed => "completed",
            TaskStatus::Skipped => "skipped",
        };

        let plan_display = plan.format_for_display();

        let result = ToolCallResult {
            content: format!("Task {} {}", task_number, action_text),
            ui_display_info: UIDisplayInfo {
                collapsed: format!("📋 Task {} {}", task_number, action_text),
                expanded: Some(format!("📋 Task {} {}\n\n{}", task_number, action_text, plan_display)),
            },
        };

        self.send_success_result(tool_call_id, result);
    }

    fn send_error_result(&self, tool_call_id: &str, error_result: ToolCallResult) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(error_result),
            },
        };

        bindings::hive::actor::messaging::broadcast(
            ToolCallStatusUpdate::MESSAGE_TYPE,
            &serde_json::to_string(&update).unwrap().into_bytes(),
        );
    }

    fn send_success_result(&self, tool_call_id: &str, result: ToolCallResult) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done {
                result: Ok(result),
            },
        };

        bindings::hive::actor::messaging::broadcast(
            ToolCallStatusUpdate::MESSAGE_TYPE,
            &serde_json::to_string(&update).unwrap().into_bytes(),
        );
    }
}
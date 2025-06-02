use serde::{Deserialize, Serialize};

use crate::actors::{ToolCallStatus, ToolCallType};

/// Tracks a tool execution with all its updates
#[derive(Debug, Clone)]
pub struct ToolExecution {
    pub call_id: String,
    pub name: String,
    pub tool_type: ToolCallType,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub updates: Vec<(chrono::DateTime<chrono::Utc>, ToolCallStatus)>,
}

impl ToolExecution {
    pub fn new(call_id: String, name: String, tool_type: ToolCallType) -> Self {
        Self {
            call_id,
            name,
            tool_type,
            start_time: chrono::Utc::now(),
            updates: Vec::new(),
        }
    }

    pub fn add_update(&mut self, status: ToolCallStatus) {
        self.updates.push((chrono::Utc::now(), status));
    }

    pub fn is_complete(&self) -> bool {
        self.updates
            .iter()
            .any(|(_, status)| matches!(status, ToolCallStatus::Finished(_)))
    }

    pub fn latest_status(&self) -> Option<&ToolCallStatus> {
        self.updates.last().map(|(_, status)| status)
    }
}

/// Events that can be displayed in the TUI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TuiEvent {
    UserInput {
        text: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    UserMicrophoneInput {
        text: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    AssistantResponse {
        text: String,
        timestamp: chrono::DateTime<chrono::Utc>,
        is_partial: bool,
    },
    Screenshot {
        name: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    ClipboardCapture {
        excerpt: String,
        full_content: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    FunctionCall {
        name: String,
        args: Option<String>,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    FunctionResult {
        name: String,
        result: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    CommandPrompt {
        command: String,
        args: Vec<String>,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    CommandResult {
        command: String,
        stdout: String,
        stderr: String,
        exit_code: i32,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    Error {
        message: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    SystemMessage {
        message: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    MicrophoneStarted {
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    MicrophoneStopped {
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    SetWaitingForResponse {
        waiting: bool,
    },
    SetWaitingForConfirmation {
        waiting: bool,
    },
    TaskPlanCreated {
        plan: crate::actors::tools::planner::TaskPlan,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    TaskPlanUpdated {
        plan: crate::actors::tools::planner::TaskPlan,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
}

impl TuiEvent {
    pub fn user_input(text: String) -> Self {
        Self::UserInput {
            text,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn user_microphone(text: String) -> Self {
        Self::UserMicrophoneInput {
            text,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn assistant_response(text: String, is_partial: bool) -> Self {
        Self::AssistantResponse {
            text,
            timestamp: chrono::Utc::now(),
            is_partial,
        }
    }

    pub fn screenshot(name: String) -> Self {
        Self::Screenshot {
            name,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn clipboard(content: String) -> Self {
        let excerpt = content
            .lines()
            .next()
            .unwrap_or(&content)
            .chars()
            .take(50)
            .collect::<String>();

        Self::ClipboardCapture {
            excerpt,
            full_content: content,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn function_call(name: String, args: Option<String>) -> Self {
        Self::FunctionCall {
            name,
            args,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn function_result(name: String, result: String) -> Self {
        Self::FunctionResult {
            name,
            result,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn command_prompt(command: String, args: Vec<String>) -> Self {
        Self::CommandPrompt {
            command,
            args,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn command_result(command: String, stdout: String, stderr: String, exit_code: i32) -> Self {
        Self::CommandResult {
            command,
            stdout,
            stderr,
            exit_code,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn error(message: String) -> Self {
        Self::Error {
            message,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn system(message: String) -> Self {
        Self::SystemMessage {
            message,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn set_waiting_for_response(waiting: bool) -> Self {
        Self::SetWaitingForResponse { waiting }
    }

    pub fn set_waiting_for_confirmation(waiting: bool) -> Self {
        Self::SetWaitingForConfirmation { waiting }
    }

    pub fn task_plan_created(plan: crate::actors::tools::planner::TaskPlan) -> Self {
        Self::TaskPlanCreated {
            plan,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn task_plan_updated(plan: crate::actors::tools::planner::TaskPlan) -> Self {
        Self::TaskPlanUpdated {
            plan,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn microphone_started() -> Self {
        Self::MicrophoneStarted {
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn microphone_stopped() -> Self {
        Self::MicrophoneStopped {
            timestamp: chrono::Utc::now(),
        }
    }
}

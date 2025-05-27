use serde::{Deserialize, Serialize};

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
        plan: crate::worker::TaskPlan,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    TaskPlanUpdated {
        plan: crate::worker::TaskPlan,
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
    
    pub fn task_plan_created(plan: crate::worker::TaskPlan) -> Self {
        Self::TaskPlanCreated {
            plan,
            timestamp: chrono::Utc::now(),
        }
    }
    
    pub fn task_plan_updated(plan: crate::worker::TaskPlan) -> Self {
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
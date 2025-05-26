use snafu::Snafu;

/// Errors while executing commands
#[derive(Debug, Snafu)]
pub enum CommandError {
    #[snafu(display("Failed to execute command: {}", command))]
    ExecutionFailed {
        command: String,
        #[snafu(source)]
        source: std::io::Error,
    },

    #[snafu(display("Failed to get command output"))]
    OutputFailed {
        #[snafu(source)]
        source: std::string::FromUtf8Error,
    },

    #[snafu(display("User denied command execution"))]
    UserDenied,

    #[snafu(display("Failed to read user input"))]
    InputFailed {
        #[snafu(source)]
        source: std::io::Error,
    },
}

type CommandResult<T> = Result<T, CommandError>;

/// Pending command execution
#[derive(Clone, Debug)]
pub struct PendingCommand {
    pub command: String,
    pub args: Vec<String>,
    pub tool_call_id: String,
}

/// Displays the command confirmation prompt
pub fn display_command_prompt(_command: &str, _args: &[String]) {
    // Command prompts are now handled by the TUI
}



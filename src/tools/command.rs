use snafu::Snafu;
use std::io::{self, Write};

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
pub fn display_command_prompt(command: &str, args: &[String]) {
    println!("\n[COMMAND EXECUTION REQUEST]");
    println!("The assistant wants to execute the following command:");
    println!("  $ {} {}", command, args.join(" "));
    println!("Allow execution? (y/n): ");
    io::stdout().flush().unwrap();
}



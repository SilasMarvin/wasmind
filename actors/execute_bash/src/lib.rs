use hive_actor_utils::{
    messages::common_messages::tools::{ExecuteTool, UIDisplayInfo},
    tools,
};

#[allow(warnings)]
mod bindings;

mod icons;

const MAX_COMMAND_OUTPUT_CHARS: usize = 16_384;
const TRUNCATION_HEAD_CHARS: usize = 4_000; // Keep first 4k chars
const TRUNCATION_TAIL_CHARS: usize = 4_000; // Keep last 4k chars

#[derive(tools::macros::Tool)]
#[tool(
    name = "execute_command",
    description = "Execute a bash command in a stateless environment. Commands are executed using 'bash -c', supporting all bash features including pipes (|), redirections (>, >>), command chaining (&&, ||), and other shell operators. Each command runs in a fresh, isolated bash environment without any session state from previous commands. Examples: echo 'test' > file.txt, ls | grep pattern, command1 && command2",
    schema = r#"{
    "type": "object",
    "properties": {
        "command": {
            "type": "string",
            "description": "The bash command to execute. Can include shell features like pipes, redirections, etc."
        },
        "args": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Additional arguments to append to the command. These will be joined with spaces."
        },
        "directory": {
            "type": "string",
            "description": "Optional directory to execute the command in. Defaults to the current working directory of not specified."
        },
        "timeout": {
            "type": "integer",
            "description": "Optional timeout in seconds. Defaults to 30 seconds if not specified. Maximum allowed is 600 seconds (10 minutes)",
            "default": 30,
            "minimum": 1,
            "maximum": 600
        }
    },
    "required": ["command"]
}"#
)]
struct CommandTool {}

impl tools::Tool for CommandTool {
    fn new() -> Self {
        Self {}
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        todo!()
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct CommandParams {
    pub command: String,
    pub args: Option<Vec<String>>,
    pub directory: Option<String>,
    pub timeout: Option<u64>,
}

/// Command execution outcome for UI display
#[derive(Debug)]
enum CommandOutcome {
    Success {
        stdout: String,
        stderr: String,
    },
    Failed {
        exit_code: i32,
        stdout: String,
        stderr: String,
    },
    Timeout,
    Signal,
    Error(String),
}

/// Format command for UI display during processing states
fn format_command_for_ui_display(command: &str, args: &[String], state: &str) -> UIDisplayInfo {
    let full_command = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };

    let collapsed = format!("{} {}:\n{}", icons::GEAR_ICON, state, full_command);
    let expanded = format!("{} {}:\n{}", icons::GEAR_ICON, state, full_command);

    UIDisplayInfo {
        collapsed,
        expanded: Some(expanded),
    }
}

/// Create UI display info for command execution
fn format_command_outcome_for_ui_display(command: &str, outcome: CommandOutcome) -> UIDisplayInfo {
    let collapsed = match &outcome {
        CommandOutcome::Success { .. } => {
            format!("{} Command succeeded:\n{}", icons::SUCCESS_ICON, command)
        }
        CommandOutcome::Failed { exit_code, .. } => {
            format!(
                "{} Command failed (exit {}):\n{}",
                icons::FAILED_ICON,
                exit_code,
                command
            )
        }
        CommandOutcome::Timeout => {
            format!("{} Command timed out:\n{}", icons::TIMEOUT_ICON, command)
        }
        CommandOutcome::Signal => format!(
            "{} Command terminated by signal:\n{}",
            icons::SIGNAL_ICON,
            command
        ),
        CommandOutcome::Error(_) => format!("{} Command error:\n{}", icons::ERROR_ICON, command),
    };

    let expanded = match &outcome {
        CommandOutcome::Success { stdout, stderr }
        | CommandOutcome::Failed { stdout, stderr, .. } => {
            format_command_output(&collapsed, stdout, stderr)
        }
        CommandOutcome::Timeout | CommandOutcome::Signal => {
            format!("{}\n(no output)", collapsed)
        }
        CommandOutcome::Error(error) => {
            format!("{}\n\n{}", collapsed, error)
        }
    };

    UIDisplayInfo {
        collapsed,
        expanded: Some(expanded),
    }
}

/// Format command output with proper stdout/stderr sections
fn format_command_output(header: &str, stdout: &str, stderr: &str) -> String {
    let mut output = header.to_string();

    if stdout.is_empty() && stderr.is_empty() {
        output.push_str("\n(no output)");
        return output;
    }

    if !stdout.is_empty() {
        output.push_str("\n\n=== stdout ===\n");
        output.push_str(stdout);
    }

    if !stderr.is_empty() {
        if !stdout.is_empty() {
            output.push('\n');
        }
        output.push_str("\n=== stderr ===\n");
        output.push_str(stderr);
    }

    output
}

/// Smart truncation that preserves the beginning and end of the output
fn smart_truncate(text: &str) -> String {
    let char_count = text.chars().count();
    if char_count <= MAX_COMMAND_OUTPUT_CHARS {
        return text.to_string();
    }

    let chars: Vec<char> = text.chars().collect();
    let head: String = chars.iter().take(TRUNCATION_HEAD_CHARS).collect();
    let tail: String = chars
        .iter()
        .skip(char_count - TRUNCATION_TAIL_CHARS)
        .collect();

    let truncated_chars = char_count - TRUNCATION_HEAD_CHARS - TRUNCATION_TAIL_CHARS;
    format!(
        "{}\n... {} characters truncated ...\n{}\n\nNote: Output was truncated. To search within the full output, try: command | rg 'pattern' or command | head -50",
        head, truncated_chars, tail
    )
}

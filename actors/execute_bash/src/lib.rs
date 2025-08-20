use wasmind_actor_utils::{
    common_messages::{
        assistant::{Section, SystemPromptContent, SystemPromptContribution},
        tools::{ExecuteTool, UIDisplayInfo},
    },
    messages::Message,
    tools,
};

#[allow(warnings)]
mod bindings;

const EXECUTE_BASH_USAGE_GUIDE: &str = r#"## execute_bash Tool - System Commands & File Exploration

**Purpose**: Execute bash commands for system tasks, builds, tests, and file exploration.

**File Exploration Best Practices**:

Use modern tools when exploring codebases:
- `rg "pattern"` instead of `grep -r` for searching file contents
- `fd filename` instead of `find -name` for locating files
- Both respect .gitignore and are much more efficient

**Common Exploration Patterns**:
```bash
# Find where a function is defined
rg "def function_name"

# Locate all test files
fd test

# Get project structure
tree -L 2 -I 'node_modules|__pycache__'

# Search with context
rg "error" -C 2

# Count occurrences
rg "TODO" --count
```

**Tips**:
- Start with `ls -la` to understand the current directory
- Use `head`/`tail` to sample large files
- Pipe to `wc -l` for line counts
- Add `--type` or `-t` flags to filter by file type"

**Output Limits**:
- Command output is limited to prevent memory issues
- Truncation is indicated when output exceeds limits
- Use pipes like `| head -100` or `| grep pattern` to filter output"#;

#[derive(tools::macros::Tool)]
#[tool(
    name = "execute_bash",
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
struct CommandTool {
    _scope: String,
}

impl tools::Tool for CommandTool {
    fn new(scope: String, _config: String) -> Self {
        // Broadcast usage guidelines for file exploration
        bindings::wasmind::actor::messaging::broadcast(
            SystemPromptContribution::MESSAGE_TYPE,
            &serde_json::to_string(&SystemPromptContribution {
                agent: scope.clone(),
                key: "execute_bash:usage_guide".to_string(),
                content: SystemPromptContent::Text(EXECUTE_BASH_USAGE_GUIDE.to_string()),
                priority: 800,
                section: Some(Section::Tools),
            })
            .unwrap()
            .into_bytes(),
        );

        Self { _scope: scope }
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        let params: CommandParams =
            match serde_json::from_str(&tool_call.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    let error_msg = format!("Failed to parse command parameters: {}", e);
                    let ui_display = UIDisplayInfo {
                        collapsed: "Parameters: Invalid format".to_string(),
                        expanded: Some(format!(
                            "Error: Failed to parse parameters\n\nDetails: {}",
                            e
                        )),
                    };

                    self.send_error_result(
                        &tool_call.tool_call.id,
                        &tool_call.originating_request_id,
                        error_msg,
                        ui_display,
                    );
                    return;
                }
            };

        // Clamp timeout between 1-600 seconds for safety
        let timeout = params.timeout.unwrap_or(30).min(600).max(1);

        let full_command = if let Some(ref args) = params.args {
            if args.is_empty() {
                params.command.clone()
            } else {
                format!("{} {}", params.command, args.join(" "))
            }
        } else {
            params.command.clone()
        };

        // Execute the command directly - no processing state in simplified version
        self.execute_command(
            &params,
            &full_command,
            timeout,
            &tool_call.tool_call.id,
            &tool_call.originating_request_id,
        );
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

/// Create UI display info for command execution
fn format_command_outcome_for_ui_display(command: &str, outcome: CommandOutcome) -> UIDisplayInfo {
    let collapsed = match &outcome {
        CommandOutcome::Success { stdout, stderr } => {
            let output_summary = if stdout.is_empty() && stderr.is_empty() {
                "No output".to_string()
            } else {
                let stdout_lines = stdout.lines().count();
                let stderr_lines = stderr.lines().count();
                if stderr_lines > 0 {
                    format!(
                        "{} lines output, {} lines stderr",
                        stdout_lines, stderr_lines
                    )
                } else {
                    format!("{} lines output", stdout_lines)
                }
            };
            format!("{}: Success ({})", command, output_summary)
        }
        CommandOutcome::Failed {
            exit_code,
            stdout,
            stderr,
        } => {
            let output_info = if stdout.is_empty() && stderr.is_empty() {
                "no output".to_string()
            } else {
                let total_lines = stdout.lines().count() + stderr.lines().count();
                format!("{} lines output", total_lines)
            };
            format!("{}: Failed (exit {}, {})", command, exit_code, output_info)
        }
        CommandOutcome::Timeout => {
            format!("{}: Timed out", command)
        }
        CommandOutcome::Signal => {
            format!("{}: Terminated by signal", command)
        }
        CommandOutcome::Error(error) => {
            format!(
                "{}: Error ({})",
                command,
                error.lines().next().unwrap_or("unknown error")
            )
        }
    };

    // For expanded view, show the full command and complete output
    let expanded = match &outcome {
        CommandOutcome::Success { stdout, stderr }
        | CommandOutcome::Failed { stdout, stderr, .. } => {
            let header = format!("Command: {}", command);
            format_command_output(&header, stdout, stderr)
        }
        CommandOutcome::Timeout => {
            format!(
                "Command: {}\n\nResult: Execution timed out (no output)",
                command
            )
        }
        CommandOutcome::Signal => {
            format!(
                "Command: {}\n\nResult: Terminated by signal (no output)",
                command
            )
        }
        CommandOutcome::Error(error) => {
            format!("Command: {}\n\nError: {}", command, error)
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

impl CommandTool {
    /// Send error result with UI display
    fn send_error_result(
        &self,
        tool_call_id: &str,
        originating_request_id: &str,
        error_msg: String,
        ui_display: UIDisplayInfo,
    ) {
        use wasmind_actor_utils::common_messages::tools::{
            ToolCallResult, ToolCallStatus, ToolCallStatusUpdate,
        };

        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: originating_request_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(ToolCallResult {
                    content: error_msg,
                    ui_display_info: ui_display,
                }),
            },
        };

        // In WASM, we use the broadcast mechanism from the generated trait
        bindings::wasmind::actor::messaging::broadcast(
            ToolCallStatusUpdate::MESSAGE_TYPE,
            &serde_json::to_string(&update).unwrap().into_bytes(),
        );
    }

    /// Send success result with UI display
    fn send_success_result(
        &self,
        tool_call_id: &str,
        originating_request_id: &str,
        result: String,
        ui_display: UIDisplayInfo,
    ) {
        use wasmind_actor_utils::common_messages::tools::{
            ToolCallResult, ToolCallStatus, ToolCallStatusUpdate,
        };

        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: originating_request_id.to_string(),
            status: ToolCallStatus::Done {
                result: Ok(ToolCallResult {
                    content: result,
                    ui_display_info: ui_display,
                }),
            },
        };

        bindings::wasmind::actor::messaging::broadcast(
            ToolCallStatusUpdate::MESSAGE_TYPE,
            &serde_json::to_string(&update).unwrap().into_bytes(),
        );
    }

    /// Execute command using the command interface
    fn execute_command(
        &self,
        params: &CommandParams,
        full_command: &str,
        timeout: u64,
        tool_call_id: &str,
        originating_request_id: &str,
    ) {
        // Create command request using bash -c for shell features
        let mut cmd = bindings::wasmind::actor::command::Cmd::new("bash");

        // Set args for bash -c command
        cmd = cmd.args(&["-c".to_string(), full_command.to_string()]);

        // Set working directory if specified
        if let Some(ref directory) = params.directory {
            cmd = cmd.current_dir(directory);
        }

        // Set timeout
        cmd = cmd.timeout(timeout as u32);

        // Set max output bytes
        cmd = cmd.max_output_bytes(5_000);

        // Execute the command
        match cmd.run() {
            Ok(output) => {
                self.handle_command_output(
                    full_command,
                    output,
                    tool_call_id,
                    originating_request_id,
                );
            }
            Err(e) => {
                let error_msg = format!("Failed to execute command '{}': {}", full_command, e);
                let ui_display = format_command_outcome_for_ui_display(
                    full_command,
                    CommandOutcome::Error(error_msg.clone()),
                );
                self.send_error_result(tool_call_id, originating_request_id, error_msg, ui_display);
            }
        }
    }

    /// Handle command output and create appropriate response
    fn handle_command_output(
        &self,
        command: &str,
        output: bindings::wasmind::actor::command::CommandOutput,
        tool_call_id: &str,
        originating_request_id: &str,
    ) {
        use bindings::wasmind::actor::command::ExitStatus;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        let outcome = match &output.status {
            ExitStatus::Exited(code) if *code == 0 => {
                // Command succeeded
                CommandOutcome::Success {
                    stdout: stdout.clone(),
                    stderr: stderr.clone(),
                }
            }
            ExitStatus::Exited(code) => {
                // Command failed with exit code
                CommandOutcome::Failed {
                    exit_code: *code as i32,
                    stdout: stdout.clone(),
                    stderr: stderr.clone(),
                }
            }
            ExitStatus::Signaled(_) => {
                // Terminated by signal
                CommandOutcome::Signal
            }
            ExitStatus::FailedToStart(_) => {
                // Failed to start
                CommandOutcome::Error("Failed to start command".to_string())
            }
            ExitStatus::TimeoutExpired => {
                // Timeout
                CommandOutcome::Timeout
            }
        };

        // Create UI display
        let ui_display = format_command_outcome_for_ui_display(command, outcome);

        // Determine result content for the LLM
        let result_content = match &output.status {
            ExitStatus::Exited(0) => {
                // Success case
                let mut result = if stdout.is_empty() && stderr.is_empty() {
                    "Command completed successfully with no output".to_string()
                } else if stderr.is_empty() {
                    stdout.clone()
                } else if stdout.is_empty() {
                    stderr.clone()
                } else {
                    format!("=== stdout ===\n{}\n\n=== stderr ===\n{}", stdout, stderr)
                };

                // Add truncation notice if needed
                if output.stdout_truncated || output.stderr_truncated {
                    result.push_str("\n\n[Output truncated - use pipes like `| head` or `| grep` to filter output]");
                }
                result
            }
            ExitStatus::Exited(exit_code) => {
                // Failure case
                let mut error_msg = if stdout.is_empty() && stderr.is_empty() {
                    format!("Command failed with exit code {} (no output)", exit_code)
                } else if !stdout.is_empty() && stderr.is_empty() {
                    format!("Command failed with exit code {}:\n{}", exit_code, stdout)
                } else if stdout.is_empty() && !stderr.is_empty() {
                    format!("Command failed with exit code {}:\n{}", exit_code, stderr)
                } else {
                    format!(
                        "Command failed with exit code {}:\n=== stdout ===\n{}\n\n=== stderr ===\n{}",
                        exit_code, stdout, stderr
                    )
                };

                // Add truncation notice if needed
                if output.stdout_truncated || output.stderr_truncated {
                    error_msg.push_str("\n\n[Output truncated - use pipes like `| head` or `| grep` to filter output]");
                }
                error_msg
            }
            ExitStatus::Signaled(signal) => {
                let mut error_msg = if stdout.is_empty() && stderr.is_empty() {
                    format!("Command terminated by signal {} (no output)", signal)
                } else {
                    format!(
                        "Command terminated by signal {}:\n=== stdout ===\n{}\n\n=== stderr ===\n{}",
                        signal, stdout, stderr
                    )
                };

                // Add truncation notice if needed
                if output.stdout_truncated || output.stderr_truncated {
                    error_msg.push_str("\n\n[Output truncated - use pipes like `| head` or `| grep` to filter output]");
                }
                error_msg
            }
            ExitStatus::FailedToStart(msg) => {
                format!("Failed to start command: {}", msg)
            }
            ExitStatus::TimeoutExpired => {
                let mut error_msg = if stdout.is_empty() && stderr.is_empty() {
                    "Command timed out (no output)".to_string()
                } else {
                    format!(
                        "Command timed out:\n=== stdout ===\n{}\n\n=== stderr ===\n{}",
                        stdout, stderr
                    )
                };

                // Add truncation notice if needed
                if output.stdout_truncated || output.stderr_truncated {
                    error_msg.push_str("\n\n[Output truncated - use pipes like `| head` or `| grep` to filter output]");
                }
                error_msg
            }
        };

        // Send appropriate result
        match &output.status {
            ExitStatus::Exited(0) => {
                self.send_success_result(
                    tool_call_id,
                    originating_request_id,
                    result_content,
                    ui_display,
                );
            }
            _ => {
                self.send_error_result(
                    tool_call_id,
                    originating_request_id,
                    result_content,
                    ui_display,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_params_deserialize_minimal() {
        let json = r#"{"command": "ls"}"#;
        let params: CommandParams = serde_json::from_str(json).unwrap();

        assert_eq!(params.command, "ls");
        assert_eq!(params.args, None);
        assert_eq!(params.directory, None);
        assert_eq!(params.timeout, None);
    }

    #[test]
    fn test_command_params_deserialize_full() {
        let json = r#"{
            "command": "ls",
            "args": ["-la", "--color"],
            "directory": "/tmp",
            "timeout": 60
        }"#;
        let params: CommandParams = serde_json::from_str(json).unwrap();

        assert_eq!(params.command, "ls");
        assert_eq!(
            params.args,
            Some(vec!["-la".to_string(), "--color".to_string()])
        );
        assert_eq!(params.directory, Some("/tmp".to_string()));
        assert_eq!(params.timeout, Some(60));
    }

    #[test]
    fn test_command_params_deserialize_invalid() {
        let json = r#"{"args": ["test"]}"#; // Missing required "command" field
        let result: Result<CommandParams, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_format_command_output_empty() {
        let result = format_command_output("Header", "", "");
        assert_eq!(result, "Header\n(no output)");
    }

    #[test]
    fn test_format_command_output_stdout_only() {
        let result = format_command_output("Header", "stdout content", "");
        assert_eq!(result, "Header\n\n=== stdout ===\nstdout content");
    }

    #[test]
    fn test_format_command_output_stderr_only() {
        let result = format_command_output("Header", "", "stderr content");
        assert_eq!(result, "Header\n=== stderr ===\nstderr content");
    }

    #[test]
    fn test_format_command_output_both() {
        let result = format_command_output("Header", "stdout content", "stderr content");
        assert_eq!(
            result,
            "Header\n\n=== stdout ===\nstdout content\n\n=== stderr ===\nstderr content"
        );
    }

    #[test]
    fn test_ui_display_success() {
        let outcome = CommandOutcome::Success {
            stdout: "file1.txt\nfile2.txt".to_string(),
            stderr: String::new(),
        };

        let display = format_command_outcome_for_ui_display("ls", outcome);
        assert!(display.collapsed.contains("Success"));
        assert!(display.collapsed.contains("ls"));

        let expanded = display.expanded.unwrap();
        assert!(expanded.contains("=== stdout ==="));
        assert!(expanded.contains("file1.txt"));
    }

    #[test]
    fn test_ui_display_failure() {
        let outcome = CommandOutcome::Failed {
            exit_code: 1,
            stdout: String::new(),
            stderr: "Permission denied".to_string(),
        };

        let display = format_command_outcome_for_ui_display("cat /etc/shadow", outcome);
        assert!(display.collapsed.contains("Failed"));
        assert!(display.collapsed.contains("exit 1"));

        let expanded = display.expanded.unwrap();
        assert!(expanded.contains("=== stderr ==="));
        assert!(expanded.contains("Permission denied"));
    }

    #[test]
    fn test_ui_display_timeout() {
        let outcome = CommandOutcome::Timeout;
        let display = format_command_outcome_for_ui_display("sleep 100", outcome);

        assert!(display.collapsed.contains("Timed out"));
        assert!(display.collapsed.contains("sleep 100"));

        let expanded = display.expanded.unwrap();
        assert!(expanded.contains("(no output)"));
    }

    #[test]
    fn test_ui_display_signal() {
        let outcome = CommandOutcome::Signal;
        let display = format_command_outcome_for_ui_display("kill -9 $$", outcome);

        assert!(display.collapsed.contains("Terminated by signal"));
    }

    #[test]
    fn test_ui_display_error() {
        let outcome = CommandOutcome::Error("Failed to spawn bash command".to_string());
        let display = format_command_outcome_for_ui_display("bad command", outcome);

        assert!(display.collapsed.contains("Error"));

        let expanded = display.expanded.unwrap();
        assert!(expanded.contains("Failed to spawn bash command"));
    }

    #[test]
    fn test_timeout_validation() {
        // Test timeout bounds in handle_call logic
        let tool_call = ExecuteTool {
            tool_call: wasmind_actor_utils::llm_client_types::ToolCall {
                id: "test-id".to_string(),
                tool_type: "function".to_string(),
                function: wasmind_actor_utils::llm_client_types::Function {
                    name: "execute_bash".to_string(),
                    arguments: r#"{"command": "echo test", "timeout": 700}"#.to_string(),
                },
                index: None,
            },
            originating_request_id: "Filler".to_string(),
        };

        let params: CommandParams =
            serde_json::from_str(&tool_call.tool_call.function.arguments).unwrap();
        let timeout = params.timeout.unwrap_or(30).min(600).max(1);

        // Should be clamped to 600
        assert_eq!(timeout, 600);
    }

    #[test]
    fn test_command_building() {
        // Test command building logic
        let params = CommandParams {
            command: "ls".to_string(),
            args: Some(vec!["-la".to_string(), "--color".to_string()]),
            directory: None,
            timeout: None,
        };

        let full_command = if let Some(ref args) = params.args {
            if args.is_empty() {
                params.command.clone()
            } else {
                format!("{} {}", params.command, args.join(" "))
            }
        } else {
            params.command.clone()
        };

        assert_eq!(full_command, "ls -la --color");
    }
}

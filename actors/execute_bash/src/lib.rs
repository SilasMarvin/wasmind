use hive_actor_utils::{
    common_messages::tools::{ExecuteTool, UIDisplayInfo},
    messages::Message,
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
    fn new(_config: String) -> Self {
        Self {}
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        // Parse command parameters from the function arguments
        let params: CommandParams =
            match serde_json::from_str(&tool_call.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    let error_msg = format!("Failed to parse command parameters: {}", e);
                    let ui_display = UIDisplayInfo {
                        collapsed: format!("{} Parameter Error", icons::ERROR_ICON),
                        expanded: Some(format!(
                            "{} Parameter Error:\n{}",
                            icons::ERROR_ICON,
                            error_msg
                        )),
                    };

                    self.send_error_result(&tool_call.tool_call.id, error_msg, ui_display);
                    return;
                }
            };

        // Validate timeout parameter
        let timeout = params.timeout.unwrap_or(30).min(600).max(1);

        // Build full command with args
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
        self.execute_command(&params, &full_command, timeout, &tool_call.tool_call.id);
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

impl CommandTool {
    /// Send error result with UI display
    fn send_error_result(&self, tool_call_id: &str, error_msg: String, ui_display: UIDisplayInfo) {
        use hive_actor_utils::common_messages::tools::{
            ToolCallResult, ToolCallStatus, ToolCallStatusUpdate,
        };

        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(ToolCallResult {
                    content: error_msg,
                    ui_display_info: ui_display,
                }),
            },
        };

        // In WASM, we use the broadcast mechanism from the generated trait
        bindings::hive::actor::messaging::broadcast(
            ToolCallStatusUpdate::MESSAGE_TYPE,
            &serde_json::to_string(&update).unwrap().into_bytes(),
        );
    }

    /// Send success result with UI display
    fn send_success_result(&self, tool_call_id: &str, result: String, ui_display: UIDisplayInfo) {
        use hive_actor_utils::common_messages::tools::{
            ToolCallResult, ToolCallStatus, ToolCallStatusUpdate,
        };

        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done {
                result: Ok(ToolCallResult {
                    content: result,
                    ui_display_info: ui_display,
                }),
            },
        };

        bindings::hive::actor::messaging::broadcast(
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
    ) {
        // Create command request using bash -c for shell features
        let mut cmd = bindings::hive::actor::command::Cmd::new("bash");

        // Set args for bash -c command
        cmd = cmd.args(&["-c".to_string(), full_command.to_string()]);

        // Set working directory if specified
        if let Some(ref directory) = params.directory {
            cmd = cmd.current_dir(directory);
        }

        // Set timeout
        cmd = cmd.timeout(timeout as u32);

        // Execute the command
        match cmd.run() {
            Ok(output) => {
                self.handle_command_output(full_command, output, tool_call_id);
            }
            Err(e) => {
                let error_msg = format!("Failed to execute command '{}': {}", full_command, e);
                let ui_display = format_command_outcome_for_ui_display(
                    full_command,
                    CommandOutcome::Error(error_msg.clone()),
                );
                self.send_error_result(tool_call_id, error_msg, ui_display);
            }
        }
    }

    /// Handle command output and create appropriate response
    fn handle_command_output(
        &self,
        command: &str,
        output: bindings::hive::actor::command::CommandOutput,
        tool_call_id: &str,
    ) {
        use bindings::hive::actor::command::ExitStatus;

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
                if stdout.is_empty() && stderr.is_empty() {
                    "Command completed successfully with no output".to_string()
                } else if stderr.is_empty() {
                    smart_truncate(&stdout)
                } else if stdout.is_empty() {
                    smart_truncate(&stderr)
                } else {
                    smart_truncate(&format!(
                        "=== stdout ===\n{}\n\n=== stderr ===\n{}",
                        stdout, stderr
                    ))
                }
            }
            ExitStatus::Exited(exit_code) => {
                // Failure case
                let error_msg = if stdout.is_empty() && stderr.is_empty() {
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
                smart_truncate(&error_msg)
            }
            ExitStatus::Signaled(signal) => {
                let error_msg = if stdout.is_empty() && stderr.is_empty() {
                    format!("Command terminated by signal {} (no output)", signal)
                } else {
                    format!(
                        "Command terminated by signal {}:\n=== stdout ===\n{}\n\n=== stderr ===\n{}",
                        signal, stdout, stderr
                    )
                };
                smart_truncate(&error_msg)
            }
            ExitStatus::FailedToStart(msg) => {
                smart_truncate(&format!("Failed to start command: {}", msg))
            }
            ExitStatus::TimeoutExpired => {
                let error_msg = if stdout.is_empty() && stderr.is_empty() {
                    "Command timed out (no output)".to_string()
                } else {
                    format!(
                        "Command timed out:\n=== stdout ===\n{}\n\n=== stderr ===\n{}",
                        stdout, stderr
                    )
                };
                smart_truncate(&error_msg)
            }
        };

        // Send appropriate result
        match &output.status {
            ExitStatus::Exited(0) => {
                self.send_success_result(tool_call_id, result_content, ui_display);
            }
            _ => {
                self.send_error_result(tool_call_id, result_content, ui_display);
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
    fn test_smart_truncate_small_output() {
        let small_text = "This is a small output";
        let result = smart_truncate(small_text);
        assert_eq!(result, small_text);
    }

    #[test]
    fn test_smart_truncate_large_output() {
        // Create a large string that exceeds MAX_COMMAND_OUTPUT_CHARS
        let large_text = "a".repeat(20_000);
        let result = smart_truncate(&large_text);

        // Check that the result contains the truncation message
        assert!(result.contains("... "));
        assert!(result.contains(" characters truncated ..."));

        // Check that it starts with the beginning of the original text
        assert!(result.starts_with(&"a".repeat(TRUNCATION_HEAD_CHARS)));

        // Check that it contains the tail portion
        assert!(result.contains(&"a".repeat(TRUNCATION_TAIL_CHARS)));

        // Check for the helpful note
        assert!(result.contains("Note: Output was truncated"));
    }

    #[test]
    fn test_smart_truncate_unicode() {
        // Test with unicode characters
        let unicode_text = "🦀".repeat(6_000); // Rust crab emoji
        let result = smart_truncate(&unicode_text);

        if unicode_text.chars().count() > MAX_COMMAND_OUTPUT_CHARS {
            assert!(result.contains("... "));
            assert!(result.contains(" characters truncated ..."));
        } else {
            assert_eq!(result, unicode_text);
        }
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
        assert!(display.collapsed.contains("✓"));
        assert!(display.collapsed.contains("Command succeeded"));
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
        assert!(display.collapsed.contains("✗"));
        assert!(display.collapsed.contains("Command failed"));
        assert!(display.collapsed.contains("exit 1"));

        let expanded = display.expanded.unwrap();
        assert!(expanded.contains("=== stderr ==="));
        assert!(expanded.contains("Permission denied"));
    }

    #[test]
    fn test_ui_display_timeout() {
        let outcome = CommandOutcome::Timeout;
        let display = format_command_outcome_for_ui_display("sleep 100", outcome);

        assert!(display.collapsed.contains("◐"));
        assert!(display.collapsed.contains("Command timed out"));
        assert!(display.collapsed.contains("sleep 100"));

        let expanded = display.expanded.unwrap();
        assert!(expanded.contains("(no output)"));
    }

    #[test]
    fn test_ui_display_signal() {
        let outcome = CommandOutcome::Signal;
        let display = format_command_outcome_for_ui_display("kill -9 $$", outcome);

        assert!(display.collapsed.contains("◆"));
        assert!(display.collapsed.contains("Command terminated by signal"));
    }

    #[test]
    fn test_ui_display_error() {
        let outcome = CommandOutcome::Error("Failed to spawn bash command".to_string());
        let display = format_command_outcome_for_ui_display("bad command", outcome);

        assert!(display.collapsed.contains("!"));
        assert!(display.collapsed.contains("Command error"));

        let expanded = display.expanded.unwrap();
        assert!(expanded.contains("Failed to spawn bash command"));
    }

    #[test]
    fn test_timeout_validation() {
        // Test timeout bounds in handle_call logic
        let tool_call = ExecuteTool {
            tool_call: hive_llm_types::types::ToolCall {
                id: "test-id".to_string(),
                tool_type: "function".to_string(),
                function: hive_llm_types::types::Function {
                    name: "execute_command".to_string(),
                    arguments: r#"{"command": "echo test", "timeout": 700}"#.to_string(),
                },
                index: None,
            },
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

use crate::llm_client::ToolCall;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use crate::actors::{
    ActorContext, ActorMessage, Message, ToolCallStatus, ToolCallUpdate, ToolDisplayInfo,
    tui::icons,
};
use crate::config::ParsedConfig;
use crate::scope::Scope;

use super::Tool;

const MAX_COMMAND_OUTPUT_CHARS: usize = 16_384;
const TRUNCATION_HEAD_CHARS: usize = 4_000; // Keep first 4k chars
const TRUNCATION_TAIL_CHARS: usize = 4_000; // Keep last 4k chars

/// Command execution outcome for TUI display
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

/// Create TUI display info for command execution
fn create_command_tui_display(command: &str, outcome: CommandOutcome) -> ToolDisplayInfo {
    let collapsed = match &outcome {
        CommandOutcome::Success { .. } => format!("{} Command succeeded:\n{}", icons::SUCCESS_ICON, command),
        CommandOutcome::Failed { exit_code, .. } => {
            format!("{} Command failed (exit {}):\n{}", icons::FAILED_ICON, exit_code, command)
        }
        CommandOutcome::Timeout => format!("{} Command timed out:\n{}", icons::TIMEOUT_ICON, command),
        CommandOutcome::Signal => format!("{} Command terminated by signal:\n{}", icons::SIGNAL_ICON, command),
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

    ToolDisplayInfo {
        collapsed,
        expanded: Some(expanded),
    }
}

/// Format command for TUI display during processing states
fn format_command_for_display(command: &str, args: &[String], state: &str) -> ToolDisplayInfo {
    let full_command = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };

    let collapsed = format!("{} {}:\n{}", icons::GEAR_ICON, state, full_command);
    let expanded = format!("{} {}:\n{}", icons::GEAR_ICON, state, full_command);

    ToolDisplayInfo {
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

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct CommandParams {
    pub command: String,
    pub args: Option<Vec<String>>,
    pub directory: Option<String>,
    pub timeout: Option<u64>,
}

/// Pending command execution
#[derive(Clone, Debug)]
pub struct PendingCommand {
    pub command: String,
    pub args: Vec<String>,
    pub directory: Option<String>,
    pub timeout: u64,
    pub tool_call_id: String,
}

/// Command actor
#[derive(wasmind_macros::ActorContext)]
pub struct CommandTool {
    tx: broadcast::Sender<ActorMessage>,
    pending_command: Option<PendingCommand>,
    config: ParsedConfig,
    running_commands: HashMap<String, JoinHandle<()>>,
    scope: Scope,
}

impl CommandTool {
    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self {
            config,
            tx,
            pending_command: None,
            running_commands: HashMap::new(),
            scope,
        }
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

    #[tracing::instrument(name = "execute_command", skip(self, args, tool_call_id, scope), fields(command = %command, args_count = args.len(), timeout = %timeout, call_id = %tool_call_id))]
    async fn execute_command(
        &mut self,
        command: &str,
        args: &[String],
        directory: Option<&str>,
        timeout: u64,
        tool_call_id: &str,
        scope: Scope,
    ) {
        let command = command.to_string();
        let args = args.to_vec();
        let directory = directory.map(String::from);
        let tool_call_id_clone = tool_call_id.to_string();
        let tx = self.tx.clone();

        // Spawn the command in a separate task
        let handle = tokio::spawn(async move {
            let tool_call_id = tool_call_id_clone;

            // Build the full command string from command and args
            let full_command = if args.is_empty() {
                command.clone()
            } else {
                format!("{} {}", command, args.join(" "))
            };

            // Always use bash -c to execute commands to support shell features
            let mut child = tokio::process::Command::new("bash");
            child
                .arg("-c")
                .arg(&full_command)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            // Set the working directory
            if let Some(dir) = directory {
                child.current_dir(dir);
            } else {
                child.current_dir(
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
                );
            }

            // Spawn the command
            let child_process = match child.spawn() {
                Ok(child) => child,
                Err(e) => {
                    let error_msg =
                        format!("Failed to spawn bash command '{}': {}", full_command, e);
                    error!("{}", error_msg);
                    let tui_display = Some(create_command_tui_display(
                        &full_command,
                        CommandOutcome::Error(error_msg.clone()),
                    ));
                    let _ = tx.send(ActorMessage {
                        scope,
                        message: Message::ToolCallUpdate(ToolCallUpdate {
                            call_id: tool_call_id,
                            status: ToolCallStatus::Finished {
                                result: Err(error_msg),
                                tui_display,
                            },
                        }),
                    });
                    return;
                }
            };

            // Execute the command with timeout
            let timeout_duration = std::time::Duration::from_secs(timeout);
            let output_future = child_process.wait_with_output();

            let output = match tokio::time::timeout(timeout_duration, output_future).await {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => {
                    let error_msg =
                        format!("Failed to execute bash command '{}': {}", full_command, e);
                    error!("{}", error_msg);
                    let tui_display = Some(create_command_tui_display(
                        &full_command,
                        CommandOutcome::Error(error_msg.clone()),
                    ));
                    let _ = tx.send(ActorMessage {
                        scope,
                        message: Message::ToolCallUpdate(ToolCallUpdate {
                            call_id: tool_call_id,
                            status: ToolCallStatus::Finished {
                                result: Err(error_msg),
                                tui_display,
                            },
                        }),
                    });
                    return;
                }
                Err(_) => {
                    // Timeout occurred
                    // Note: The process will be automatically killed when it goes out of scope
                    let error_msg = format!(
                        "Bash command '{}' timed out after {} seconds",
                        full_command, timeout
                    );
                    error!("{}", error_msg);
                    let tui_display = Some(create_command_tui_display(
                        &full_command,
                        CommandOutcome::Timeout,
                    ));
                    let _ = tx.send(ActorMessage {
                        scope,
                        message: Message::ToolCallUpdate(ToolCallUpdate {
                            call_id: tool_call_id,
                            status: ToolCallStatus::Finished {
                                result: Err(error_msg),
                                tui_display,
                            },
                        }),
                    });
                    return;
                }
            };

            // Convert output to strings
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            // Create command outcome for TUI display
            let outcome = if output.status.success() {
                CommandOutcome::Success {
                    stdout: stdout.clone(),
                    stderr: stderr.clone(),
                }
            } else if let Some(exit_code) = output.status.code() {
                CommandOutcome::Failed {
                    exit_code,
                    stdout: stdout.clone(),
                    stderr: stderr.clone(),
                }
            } else {
                CommandOutcome::Signal
            };

            // Check if the command was successful
            let result = if output.status.success() {
                // Command succeeded
                let output_text = if stdout.is_empty() && stderr.is_empty() {
                    "Command completed successfully with no output".to_string()
                } else if stderr.is_empty() {
                    // Only stdout
                    stdout.to_string()
                } else if stdout.is_empty() {
                    // Only stderr (even on success, some commands write to stderr)
                    stderr.to_string()
                } else {
                    // Both stdout and stderr - combine them with labels
                    // Note: This doesn't preserve exact interleaving, but provides both
                    format!("=== stdout ===\n{}\n\n=== stderr ===\n{}", stdout, stderr)
                };
                Ok(CommandTool::smart_truncate(&output_text))
            } else {
                // Command failed
                let error_msg = if let Some(code) = output.status.code() {
                    // Include both stdout and stderr for failed commands
                    if stdout.is_empty() && stderr.is_empty() {
                        format!("Command failed with exit code {} (no output)", code)
                    } else if !stdout.is_empty() && stderr.is_empty() {
                        format!("Command failed with exit code {}:\n{}", code, stdout)
                    } else if stdout.is_empty() && !stderr.is_empty() {
                        format!("Command failed with exit code {}:\n{}", code, stderr)
                    } else {
                        // Both stdout and stderr present
                        format!(
                            "Command failed with exit code {}:\n=== stdout ===\n{}\n\n=== stderr ===\n{}",
                            code, stdout, stderr
                        )
                    }
                } else {
                    // Terminated by signal
                    if stdout.is_empty() && stderr.is_empty() {
                        "Command terminated by signal (no output)".to_string()
                    } else if !stdout.is_empty() && stderr.is_empty() {
                        format!("Command terminated by signal:\n{}", stdout)
                    } else if stdout.is_empty() && !stderr.is_empty() {
                        format!("Command terminated by signal:\n{}", stderr)
                    } else {
                        // Both stdout and stderr present
                        format!(
                            "Command terminated by signal:\n=== stdout ===\n{}\n\n=== stderr ===\n{}",
                            stdout, stderr
                        )
                    }
                };
                Err(CommandTool::smart_truncate(&error_msg))
            };

            let tui_display = Some(create_command_tui_display(&full_command, outcome));

            let _ = tx.send(ActorMessage {
                scope,
                message: Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call_id,
                    status: ToolCallStatus::Finished {
                        result,
                        tui_display,
                    },
                }),
            });
        });

        // Store the handle so we can cancel it later if needed
        self.running_commands
            .insert(tool_call_id.to_string(), handle);
    }

    fn cleanup_completed_commands(&mut self) {
        // Remove completed tasks from the HashMap
        self.running_commands
            .retain(|_, handle| !handle.is_finished());
    }

    #[allow(dead_code)]
    fn cancel_command(&mut self, tool_call_id: &str) {
        if let Some(handle) = self.running_commands.remove(tool_call_id) {
            handle.abort();
            info!("Cancelled command execution for tool call {}", tool_call_id);
        }
    }

    fn cancel_all_commands(&mut self) {
        for (tool_call_id, handle) in self.running_commands.drain() {
            handle.abort();
            info!("Cancelled command execution for tool call {}", tool_call_id);
        }
    }
}

#[async_trait::async_trait]
impl Tool for CommandTool {
    const TOOL_NAME: &str = "execute_command";
    const TOOL_DESCRIPTION: &str = "Execute a bash command in a stateless environment. Commands are executed using 'bash -c', supporting all bash features including pipes (|), redirections (>, >>), command chaining (&&, ||), and other shell operators. Each command runs in a fresh, isolated bash environment without any session state from previous commands. Examples: echo 'test' > file.txt, ls | grep pattern, command1 && command2";
    const TOOL_INPUT_SCHEMA: &str = r#"{
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
}"#;

    type Params = CommandParams;

    fn awaiting_user_confirmation(&self) -> Option<&str> {
        self.pending_command
            .as_ref()
            .map(|cmd| cmd.tool_call_id.as_str())
    }

    fn create_received_tui_display(&self, _tool_call: &ToolCall, params: &Self::Params) -> Option<ToolDisplayInfo> {
        let args = params.args.as_deref().unwrap_or(&[]);
        Some(format_command_for_display(&params.command, args, "Executing"))
    }

    async fn execute_tool_call(&mut self, tool_call: ToolCall, params: Self::Params) {
        // Cleanup completed commands periodically
        self.cleanup_completed_commands();

        // Extract command and arguments from structured params
        let command = &params.command;
        let args_array = params.args.unwrap_or_default();
        let directory = params.directory;

        // Parse timeout parameter with default of 30 seconds, max 600 seconds (10 minutes)
        let timeout = params.timeout.unwrap_or(30).min(600).max(1);

        // Check if command is whitelisted
        debug!("Checking if command '{}' is whitelisted", command);
        debug!(
            "Whitelisted commands: {:?}",
            self.config.whitelisted_commands
        );

        let is_whitelisted = self.config.whitelisted_commands.iter().any(|wc| {
            // Exact match
            if wc == command {
                return true;
            }
            // Check if the command is a path that ends with the whitelisted command
            // e.g., "/usr/bin/pwd" matches "pwd"
            if command.split('/').last() == Some(wc) {
                return true;
            }
            false
        });

        if is_whitelisted {
            self.execute_command(
                command,
                &args_array,
                directory.as_deref(),
                timeout,
                &tool_call.id,
                self.scope.clone(),
            )
            .await;
        } else if self.config.auto_approve_commands {
            // Auto-approve non-whitelisted commands
            info!("Auto-approving non-whitelisted command: {}", command);
            self.execute_command(
                command,
                &args_array,
                directory.as_deref(),
                timeout,
                &tool_call.id,
                self.scope.clone(),
            )
            .await;
        } else {
            self.pending_command = Some(PendingCommand {
                command: command.to_string(),
                args: args_array.clone(),
                directory,
                timeout,
                tool_call_id: tool_call.id.clone(),
            });

            let tui_display = Some(format_command_for_display(
                command,
                &args_array,
                "Awaiting confirmation"
            ));

            self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call.id,
                status: ToolCallStatus::AwaitingUserYNConfirmation {
                    tui_display,
                },
            }));
        }
    }

    async fn handle_user_confirmed(&mut self) {
        if let Some(pending_command) = self.pending_command.take() {
            self.execute_command(
                &pending_command.command,
                &pending_command.args,
                pending_command.directory.as_deref(),
                pending_command.timeout,
                &pending_command.tool_call_id,
                self.scope.clone(),
            )
            .await
        }
    }

    async fn handle_user_denied(&mut self) {
        self.pending_command = None;
    }

    async fn handle_cancel(&mut self) {
        // Cancel all running commands
        self.cancel_all_commands();
        // Also cleanup completed commands
        self.cleanup_completed_commands();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use tempfile::TempDir;
    use tokio::process::Command as TokioCommand;
    use tokio::sync::broadcast;

    fn create_test_command() -> CommandTool {
        let (tx, _) = broadcast::channel(100);
        let config = crate::config::Config::new(true)
            .unwrap()
            .try_into()
            .unwrap();
        let scope = Scope::new();
        CommandTool::new(config, tx, scope)
    }

    #[test]
    fn test_command_deserialize_params_success() {
        let command = create_test_command();
        let json_input = r#"{
            "command": "ls -la",
            "args": ["-h", "--color"],
            "directory": "/tmp",
            "timeout": 60
        }"#;

        let result = command.deserialize_params(json_input);
        assert!(result.is_ok());

        let params = result.unwrap();
        assert_eq!(params.command, "ls -la");
        assert_eq!(
            params.args,
            Some(vec!["-h".to_string(), "--color".to_string()])
        );
        assert_eq!(params.directory, Some("/tmp".to_string()));
        assert_eq!(params.timeout, Some(60));
    }

    #[test]
    fn test_command_deserialize_params_minimal() {
        let command = create_test_command();
        let json_input = r#"{"command": "pwd"}"#;

        let result = command.deserialize_params(json_input);
        assert!(result.is_ok());

        let params = result.unwrap();
        assert_eq!(params.command, "pwd");
        assert_eq!(params.args, None);
        assert_eq!(params.directory, None);
        assert_eq!(params.timeout, None);
    }

    #[test]
    fn test_command_deserialize_params_failure() {
        let command = create_test_command();
        let json_input = r#"{"args": ["test"]}"#; // Missing required "command" field

        let result = command.deserialize_params(json_input);
        assert!(result.is_err());
    }

    #[test]
    fn test_smart_truncate_small_output() {
        let small_text = "This is a small output";
        let result = CommandTool::smart_truncate(small_text);
        assert_eq!(result, small_text);
    }

    #[test]
    fn test_smart_truncate_large_output() {
        // Create a large string that exceeds MAX_COMMAND_OUTPUT_CHARS
        let large_text = "a".repeat(20_000);
        let result = CommandTool::smart_truncate(&large_text);

        // Check that the result contains the truncation message
        assert!(result.contains("... "));
        assert!(result.contains(" characters truncated ..."));

        // Check that it starts with the beginning of the original text
        assert!(result.starts_with(&"a".repeat(TRUNCATION_HEAD_CHARS)));

        // Check that it contains the tail portion before the note
        assert!(result.contains(&"a".repeat(TRUNCATION_TAIL_CHARS)));

        // Check total length is reasonable
        assert!(result.len() < MAX_COMMAND_OUTPUT_CHARS + 200); // +200 for truncation message and note

        // Check for the helpful note
        assert!(result.contains("Note: Output was truncated"));
    }

    #[test]
    fn test_smart_truncate_unicode() {
        // Test with unicode characters
        let unicode_text = "🦀".repeat(10_000); // Rust crab emoji
        let result = CommandTool::smart_truncate(&unicode_text);

        if unicode_text.chars().count() > MAX_COMMAND_OUTPUT_CHARS {
            assert!(result.contains("... "));
            assert!(result.contains(" characters truncated ..."));
        } else {
            assert_eq!(result, unicode_text);
        }
    }

    #[test]
    fn test_smart_truncate_multiline() {
        // Test with multiline output
        let lines: Vec<String> = (0..5000).map(|i| format!("Line {}", i)).collect();
        let multiline_text = lines.join("\n");
        let result = CommandTool::smart_truncate(&multiline_text);

        if multiline_text.chars().count() > MAX_COMMAND_OUTPUT_CHARS {
            // Should preserve beginning lines
            assert!(result.starts_with("Line 0"));
            // Should preserve ending lines
            assert!(result.contains("Line 4999"));
            // Should have truncation indicator
            assert!(result.contains("... "));
        }
    }

    #[tokio::test]
    async fn test_command_runs_in_current_directory() {
        // Test that commands execute in the current working directory
        let current_dir = env::current_dir().expect("Failed to get current dir");

        // Run pwd command with current_dir set
        let mut child = TokioCommand::new("pwd");
        child.current_dir(&current_dir);

        let output = child.output().await.expect("Failed to execute pwd");
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Should contain the current directory path
        assert!(stdout.trim().contains(current_dir.to_str().unwrap()));
    }

    #[tokio::test]
    async fn test_touch_command_creates_file_in_current_dir() {
        // Create a temp directory for testing
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let temp_path = temp_dir.path();

        // Run touch command with current_dir set
        let test_file = "test_file.txt";
        let mut child = TokioCommand::new("touch");
        child.arg(test_file).current_dir(temp_path);

        let output = child.output().await.expect("Failed to execute touch");
        assert!(output.status.success(), "Touch command should succeed");

        // Verify file was created in the correct directory
        let expected_path = temp_path.join(test_file);
        assert!(
            expected_path.exists(),
            "File should exist in temp directory"
        );
        // No cleanup needed - tempfile handles it
    }

    #[tokio::test]
    async fn test_command_with_current_dir_uses_process_cwd() {
        // Our implementation should use std::env::current_dir()
        let current_dir = env::current_dir().expect("Failed to get current dir");

        // Run pwd command using our approach
        let mut child = TokioCommand::new("pwd");
        child
            .current_dir(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));

        let output = child.output().await.expect("Failed to execute pwd");
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Should match the actual current directory
        assert_eq!(stdout.trim(), current_dir.to_str().unwrap());
    }

    #[tokio::test]
    async fn test_command_with_custom_directory() {
        // Test that commands can run in a specified directory
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let temp_path = temp_dir.path();

        // Create a test file in the temp directory
        let test_file = "custom_dir_test.txt";
        std::fs::write(temp_path.join(test_file), "test content")
            .expect("Failed to write test file");

        // Run ls command in the temp directory
        let mut child = TokioCommand::new("ls");
        child.current_dir(temp_path);

        let output = child.output().await.expect("Failed to execute ls");
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Should contain our test file
        assert!(
            stdout.contains(test_file),
            "ls output should contain test file"
        );
        // No cleanup needed - tempfile handles it
    }

    #[tokio::test]
    async fn test_bash_c_environment_inheritance() {
        // Set a test environment variable
        unsafe {
            env::set_var("WASMIND_TEST_VAR", "test_value_123");
        }

        // Run bash -c to echo the environment variable
        let mut child = TokioCommand::new("bash");
        child.args(&["-c", "echo $WASMIND_TEST_VAR"]);

        let output = child
            .output()
            .await
            .expect("Failed to execute bash command");
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Should contain our test value
        assert_eq!(stdout.trim(), "test_value_123");

        // Clean up
        unsafe {
            env::remove_var("WASMIND_TEST_VAR");
        }
    }

    #[tokio::test]
    async fn test_bash_c_shell_redirection() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let temp_path = temp_dir.path();
        let test_file = temp_path.join("redirect_test.txt");

        // Run bash -c with redirection
        let mut child = TokioCommand::new("bash");
        child.args(&["-c", "echo 'Hello from bash' > redirect_test.txt"]);
        child.current_dir(temp_path);

        let output = child
            .output()
            .await
            .expect("Failed to execute bash command");
        assert!(output.status.success());

        // Verify file was created with correct content
        let content = std::fs::read_to_string(&test_file).expect("Failed to read redirected file");
        assert_eq!(content.trim(), "Hello from bash");
    }

    #[tokio::test]
    async fn test_bash_c_piping() {
        // Test piping commands
        let mut child = TokioCommand::new("bash");
        child.args(&["-c", "echo 'line1\nline2\nline3' | grep line2"]);

        let output = child
            .output()
            .await
            .expect("Failed to execute bash command");
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(output.status.success());
        assert_eq!(stdout.trim(), "line2");
    }

    #[tokio::test]
    async fn test_bash_c_command_chaining() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let temp_path = temp_dir.path();

        // Test && operator
        let mut child = TokioCommand::new("bash");
        child.args(&[
            "-c",
            "touch file1.txt && touch file2.txt && echo 'both created'",
        ]);
        child.current_dir(temp_path);

        let output = child
            .output()
            .await
            .expect("Failed to execute bash command");
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(output.status.success());
        assert_eq!(stdout.trim(), "both created");
        assert!(temp_path.join("file1.txt").exists());
        assert!(temp_path.join("file2.txt").exists());
    }

    #[test]
    fn test_tui_display_success() {
        let outcome = CommandOutcome::Success {
            stdout: "total 16\ndrwxr-xr-x  4 user  staff  128 Jan  1 12:00 .\ndrwxr-xr-x 10 user  staff  320 Jan  1 11:00 ..".to_string(),
            stderr: String::new(),
        };

        let display = create_command_tui_display("ls -la", outcome);
        assert_eq!(display.collapsed, "✓ Command succeeded: ls -la");
        let expanded = display.expanded.unwrap();
        assert!(expanded.contains("=== stdout ==="));
        assert!(expanded.contains("total 16"));
    }

    #[test]
    fn test_tui_display_failure() {
        let outcome = CommandOutcome::Failed {
            exit_code: 1,
            stdout: String::new(),
            stderr: "cat: /nonexistent: No such file or directory".to_string(),
        };

        let display = create_command_tui_display("cat /nonexistent", outcome);
        assert_eq!(
            display.collapsed,
            "✗ Command failed (exit 1): cat /nonexistent"
        );
        let expanded = display.expanded.unwrap();
        assert!(expanded.contains("=== stderr ==="));
        assert!(expanded.contains("No such file or directory"));
    }

    #[test]
    fn test_tui_display_timeout() {
        let outcome = CommandOutcome::Timeout;
        let display = create_command_tui_display("sleep 100", outcome);
        assert_eq!(display.collapsed, "Command timed out: sleep 100");
        let expanded = display.expanded.unwrap();
        assert!(expanded.contains("Command timed out"));
        assert!(expanded.contains("(no output)"));
    }

    #[test]
    fn test_tui_display_signal() {
        let outcome = CommandOutcome::Signal;
        let display = create_command_tui_display("kill -9 $$", outcome);
        assert_eq!(
            display.collapsed,
            "Command terminated by signal: kill -9 $$"
        );
    }

    #[test]
    fn test_tui_display_error() {
        let outcome = CommandOutcome::Error("Failed to spawn bash command".to_string());
        let display = create_command_tui_display("bad command", outcome);
        assert_eq!(display.collapsed, "Command error: bad command");
        assert!(
            display
                .expanded
                .unwrap()
                .contains("Failed to spawn bash command")
        );
    }
}

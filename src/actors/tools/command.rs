use genai::chat::{Tool, ToolCall};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use crate::actors::{
    Action, Actor, ActorMessage, Message, ToolCallStatus, ToolCallType, ToolCallUpdate,
};
use crate::config::ParsedConfig;
use crate::scope::Scope;

const MAX_COMMAND_OUTPUT_CHARS: usize = 16_384;
const TRUNCATION_HEAD_CHARS: usize = 4_000; // Keep first 4k chars
const TRUNCATION_TAIL_CHARS: usize = 4_000; // Keep last 4k chars

pub const TOOL_NAME: &str = "execute_command";
pub const TOOL_DESCRIPTION: &str = "Execute a shell command with specified arguments in a stateless environment. Each command runs in a fresh, isolated environment without any session state from previous commands. E.G. pwd, git, ls, etc...";
pub const TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "command": {
            "type": "string",
            "description": "The shell command to execute"
        },
        "args": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Arguments to pass to the shell command"
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
pub struct Command {
    tx: broadcast::Sender<ActorMessage>,
    pending_command: Option<PendingCommand>,
    config: ParsedConfig,
    running_commands: HashMap<String, JoinHandle<()>>,
    scope: Scope,
}

impl Command {
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

    #[tracing::instrument(name = "command_tool_call", skip(self, tool_call), fields(call_id = %tool_call.call_id, function = %tool_call.fn_name))]
    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        if tool_call.fn_name != TOOL_NAME {
            return;
        }

        // Parse the arguments
        let args = serde_json::from_value::<serde_json::Value>(tool_call.fn_arguments).unwrap();

        // Extract command and arguments
        let command = args.get("command").and_then(|v| v.as_str()).unwrap();

        let args_array = match args.get("args") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<String>>(),
            _ => Vec::new(),
        };

        let directory = args
            .get("directory")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Parse timeout parameter with default of 30 seconds, max 600 seconds (10 minutes)
        let timeout = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(30)
            .min(600)
            .max(1);

        let args_string = args_array.join(" ");
        let friendly_command_display = format!("{command} {args_string}");
        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id.clone(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::Command,
                friendly_command_display,
            },
        }));

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
                &command,
                &args_array,
                directory.as_deref(),
                timeout,
                &tool_call.call_id,
                self.scope.clone(),
            )
            .await;
        } else if self.config.auto_approve_commands {
            // Auto-approve non-whitelisted commands
            info!("Auto-approving non-whitelisted command: {}", command);
            self.execute_command(
                &command,
                &args_array,
                directory.as_deref(),
                timeout,
                &tool_call.call_id,
                self.scope.clone(),
            )
            .await;
        } else {
            // Await user confirmation (traditional behavior)
            self.pending_command = Some(PendingCommand {
                command: command.to_string(),
                args: args_array.clone(),
                directory,
                timeout,
                tool_call_id: tool_call.call_id.clone(),
            });

            self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call.call_id,
                status: ToolCallStatus::AwaitingUserYNConfirmation,
            }));
        }
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
            // Create the command
            let mut child = tokio::process::Command::new(&command);
            child
                .args(&args)
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
                    let error_msg = format!("Failed to spawn command '{}': {}", command, e);
                    error!("{}", error_msg);
                    let _ = tx.send(ActorMessage {
                        scope,
                        message: Message::ToolCallUpdate(ToolCallUpdate {
                            call_id: tool_call_id,
                            status: ToolCallStatus::Finished(Err(error_msg)),
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
                    let error_msg = format!("Failed to execute command '{}': {}", command, e);
                    error!("{}", error_msg);
                    let _ = tx.send(ActorMessage {
                        scope,
                        message: Message::ToolCallUpdate(ToolCallUpdate {
                            call_id: tool_call_id,
                            status: ToolCallStatus::Finished(Err(error_msg)),
                        }),
                    });
                    return;
                }
                Err(_) => {
                    // Timeout occurred
                    // Note: The process will be automatically killed when it goes out of scope
                    let error_msg =
                        format!("Command '{}' timed out after {} seconds", command, timeout);
                    error!("{}", error_msg);
                    let _ = tx.send(ActorMessage {
                        scope,
                        message: Message::ToolCallUpdate(ToolCallUpdate {
                            call_id: tool_call_id,
                            status: ToolCallStatus::Finished(Err(error_msg)),
                        }),
                    });
                    return;
                }
            };

            // Convert output to strings
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

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
                Ok(Command::smart_truncate(&output_text))
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
                Err(Command::smart_truncate(&error_msg))
            };

            let _ = tx.send(ActorMessage {
                scope,
                message: Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call_id,
                    status: ToolCallStatus::Finished(result),
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
impl Actor for Command {
    const ACTOR_ID: &'static str = "command";

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    fn get_scope(&self) -> &Scope {
        &self.scope
    }

    async fn on_start(&mut self) {
        let tool = Tool {
            name: TOOL_NAME.to_string(),
            description: Some(TOOL_DESCRIPTION.to_string()),
            schema: Some(serde_json::from_str(TOOL_INPUT_SCHEMA).unwrap()),
        };

        self.broadcast(Message::ToolsAvailable(vec![tool]));
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        // Cleanup completed commands periodically
        self.cleanup_completed_commands();

        match message.message {
            Message::AssistantToolCall(tool_call) => self.handle_tool_call(tool_call).await,
            Message::ToolCallUpdate(update) => match update.status {
                crate::actors::ToolCallStatus::ReceivedUserYNConfirmation(confirmation) => {
                    if !confirmation {
                        self.pending_command = None;
                        return;
                    }

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
                _ => (),
            },
            Message::Action(Action::Cancel) => {
                // Cancel all running commands
                self.cancel_all_commands();
            }
            _ => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use tokio::process::Command as TokioCommand;

    #[test]
    fn test_smart_truncate_small_output() {
        let small_text = "This is a small output";
        let result = Command::smart_truncate(small_text);
        assert_eq!(result, small_text);
    }

    #[test]
    fn test_smart_truncate_large_output() {
        // Create a large string that exceeds MAX_COMMAND_OUTPUT_CHARS
        let large_text = "a".repeat(20_000);
        let result = Command::smart_truncate(&large_text);

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
        let result = Command::smart_truncate(&unicode_text);

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
        let result = Command::smart_truncate(&multiline_text);

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
        let temp_dir = env::temp_dir().join(format!("hive_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temp_dir).expect("Failed to create temp dir");

        // Run touch command with current_dir set
        let test_file = "test_file.txt";
        let mut child = TokioCommand::new("touch");
        child.arg(test_file).current_dir(&temp_dir);

        let output = child.output().await.expect("Failed to execute touch");
        assert!(output.status.success(), "Touch command should succeed");

        // Verify file was created in the correct directory
        let expected_path = temp_dir.join(test_file);
        assert!(
            expected_path.exists(),
            "File should exist in temp directory"
        );

        // Cleanup
        std::fs::remove_dir_all(&temp_dir).ok();
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
        let temp_dir = env::temp_dir().join(format!("hive_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temp_dir).expect("Failed to create temp dir");

        // Create a test file in the temp directory
        let test_file = "custom_dir_test.txt";
        std::fs::write(temp_dir.join(test_file), "test content")
            .expect("Failed to write test file");

        // Run ls command in the temp directory
        let mut child = TokioCommand::new("ls");
        child.current_dir(&temp_dir);

        let output = child.output().await.expect("Failed to execute ls");
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Should contain our test file
        assert!(
            stdout.contains(test_file),
            "ls output should contain test file"
        );

        // Cleanup
        std::fs::remove_dir_all(&temp_dir).ok();
    }
}

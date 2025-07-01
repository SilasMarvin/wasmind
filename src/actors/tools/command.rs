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

pub const TOOL_NAME: &str = "execute_command";
pub const TOOL_DESCRIPTION: &str =
    "Execute a shell command with specified arguments. E.G. pwd, git, ls, etc...";
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
            "description": "Optional directory to execute the command in. If not specified, the command will run in the current working directory"
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

        let directory = args.get("directory").and_then(|v| v.as_str()).map(String::from);

        let args_string = args_array.join(" ");
        let friendly_command_display = format!("{command} {args_string}");
        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
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
                tool_call_id: tool_call.call_id.clone(),
            });

            let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call.call_id,
                status: ToolCallStatus::AwaitingUserYNConfirmation,
            }));
        }
    }

    #[tracing::instrument(name = "execute_command", skip(self, args, tool_call_id, scope), fields(command = %command, args_count = args.len(), call_id = %tool_call_id))]
    async fn execute_command(
        &mut self,
        command: &str,
        args: &[String],
        directory: Option<&str>,
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
                child.current_dir(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
            }

            // Execute the command
            let output = match child.output().await {
                Ok(output) => output,
                Err(e) => {
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
                    stdout.to_string()
                } else if stdout.is_empty() {
                    stderr.to_string()
                } else {
                    format!("STDOUT:\n{}\n\nSTDERR:\n{}", stdout, stderr)
                };
                if output_text.chars().count() > MAX_COMMAND_OUTPUT_CHARS {
                    Ok(format!(
                        "WARNING: OUTPUT WAS TOO LARGE - TRUNCATED\n\n{}",
                        output_text
                            .chars()
                            .take(MAX_COMMAND_OUTPUT_CHARS)
                            .collect::<String>()
                    ))
                } else {
                    Ok(output_text)
                }
            } else {
                // Command failed
                let error_msg = if let Some(code) = output.status.code() {
                    if stderr.is_empty() {
                        format!("Command failed with exit code {}", code)
                    } else {
                        format!("Command failed with exit code {}:\n{}", code, stderr)
                    }
                } else {
                    if stderr.is_empty() {
                        "Command terminated by signal".to_string()
                    } else {
                        format!("Command terminated by signal:\n{}", stderr)
                    }
                };
                if error_msg.chars().count() > MAX_COMMAND_OUTPUT_CHARS {
                    Err(format!(
                        "WARNING: OUTPUT WAS TOO LARGE - TRUNCATED\n\n{}",
                        error_msg
                            .chars()
                            .take(MAX_COMMAND_OUTPUT_CHARS)
                            .collect::<String>()
                    ))
                } else {
                    Err(error_msg)
                }
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

        let _ = self.broadcast(Message::ToolsAvailable(vec![tool]));
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
    use tokio::process::Command as TokioCommand;
    use std::env;
    
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
        child
            .arg(test_file)
            .current_dir(&temp_dir);
        
        let output = child.output().await.expect("Failed to execute touch");
        assert!(output.status.success(), "Touch command should succeed");
        
        // Verify file was created in the correct directory
        let expected_path = temp_dir.join(test_file);
        assert!(expected_path.exists(), "File should exist in temp directory");
        
        // Cleanup
        std::fs::remove_dir_all(&temp_dir).ok();
    }
    
    #[tokio::test]
    async fn test_command_with_current_dir_uses_process_cwd() {
        // Our implementation should use std::env::current_dir()
        let current_dir = env::current_dir().expect("Failed to get current dir");
        
        // Run pwd command using our approach
        let mut child = TokioCommand::new("pwd");
        child.current_dir(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
        
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
        std::fs::write(temp_dir.join(test_file), "test content").expect("Failed to write test file");
        
        // Run ls command in the temp directory
        let mut child = TokioCommand::new("ls");
        child.current_dir(&temp_dir);
        
        let output = child.output().await.expect("Failed to execute ls");
        let stdout = String::from_utf8_lossy(&output.stdout);
        
        // Should contain our test file
        assert!(stdout.contains(test_file), "ls output should contain test file");
        
        // Cleanup
        std::fs::remove_dir_all(&temp_dir).ok();
    }
}

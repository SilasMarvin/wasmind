use genai::chat::{Tool, ToolCall};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::actors::{
    Action, Actor, ActorMessage, Message, ToolCallStatus, ToolCallType, ToolCallUpdate,
};
use crate::config::ParsedConfig;

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
        }
    },
    "required": ["command"]
}"#;

/// Pending command execution
#[derive(Clone, Debug)]
pub struct PendingCommand {
    pub command: String,
    pub args: Vec<String>,
    pub tool_call_id: String,
}

/// Command actor
pub struct Command {
    tx: broadcast::Sender<ActorMessage>,
    pending_command: Option<PendingCommand>,
    config: ParsedConfig,
    running_commands: HashMap<String, JoinHandle<()>>,
    scope: Uuid,
}

impl Command {
    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Uuid) -> Self {
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
                &tool_call.call_id,
                self.scope.clone(),
            )
            .await;
        } else {
            // Await user confirmation (traditional behavior)
            self.pending_command = Some(PendingCommand {
                command: command.to_string(),
                args: args_array.clone(),
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
        tool_call_id: &str,
        scope: Uuid,
    ) {
        let command = command.to_string();
        let args = args.to_vec();
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

    fn get_scope(&self) -> &Uuid {
        &self.scope
    }

    async fn on_start(&mut self) {
        info!("Command tool starting - broadcasting availability");

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

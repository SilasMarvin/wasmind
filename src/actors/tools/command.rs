use genai::chat::{Tool, ToolCall};
use std::process::Stdio;
use tokio::sync::broadcast;
use tracing::{debug, info};

use crate::actors::{Actor, Message, ToolCallStatus, ToolCallType, ToolCallUpdate};
use crate::config::ParsedConfig;

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
    tx: broadcast::Sender<Message>,
    pending_command: Option<PendingCommand>,
    config: ParsedConfig,
}

impl Command {
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
        let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
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
            self.execute_command(&command, &args_array, &tool_call.call_id)
                .await;
        } else {
            self.pending_command = Some(PendingCommand {
                command: command.to_string(),
                args: args_array.clone(),
                tool_call_id: tool_call.call_id.clone(),
            });

            let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call.call_id,
                status: ToolCallStatus::AwaitingUserYNConfirmation,
            }));
        }
    }

    async fn execute_command(&mut self, command: &str, args: &[String], tool_call_id: &str) {
        // TODO: Spawn new tokio task with the ability to cancel it

        // Spawn the command
        let mut child = tokio::process::Command::new(command);
        child
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Wait for the command to complete
        let output = child.output().await.unwrap();

        // Convert output to string
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();

        // This can for sure be displayed better to the LLM
        let output = format!("STDERR: {stderr}\n\nSTDOUT: {stdout}");

        let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status: ToolCallStatus::Finished(Ok(output)),
        }));
    }
}

#[async_trait::async_trait]
impl Actor for Command {
    fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self {
        Self {
            config,
            tx,
            pending_command: None,
        }
    }

    fn get_rx(&self) -> broadcast::Receiver<Message> {
        self.tx.subscribe()
    }

    async fn on_start(&mut self) {
        info!("Command tool starting - broadcasting availability");
        
        let tool = Tool {
            name: TOOL_NAME.to_string(),
            description: Some(TOOL_DESCRIPTION.to_string()),
            schema: Some(serde_json::from_str(TOOL_INPUT_SCHEMA).unwrap()),
        };
        
        let _ = self.tx.send(Message::ToolsAvailable(vec![tool]));
    }

    async fn handle_message(&mut self, message: Message) {
        match message {
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
                        )
                        .await
                    }
                }
                _ => (),
            },
            _ => (),
        }
    }
}

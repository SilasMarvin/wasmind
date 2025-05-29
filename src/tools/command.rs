use std::collections::HashMap;
use std::process::{Command as StdCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use crossbeam::channel::{Receiver, Sender, unbounded};
use genai::chat::{ToolCall, ToolResponse};
use serde_json::Value;
use snafu::{ResultExt, Snafu};
use tracing::{debug, error};

use crate::{
    config::ParsedConfig,
    tui,
    tools::CommandExecutionStage,
    worker::Event,
};

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

/// Errors while executing commands
#[derive(Debug, Snafu)]
pub enum CommandExecutorError {
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

    #[snafu(display("Command was cancelled"))]
    Cancelled,
}

type CommandExecutorResult<T> = Result<T, CommandExecutorError>;

/// Pending command execution
#[derive(Clone, Debug)]
pub struct PendingCommand {
    pub command: String,
    pub args: Vec<String>,
    pub tool_call_id: String,
}

/// Tasks the command executor can receive
#[derive(Debug, Clone)]
enum Task {
    Execute {
        command: String,
        args: Vec<String>,
        tool_call_id: String,
    },
    Cancel {
        tool_call_id: String,
    },
}

/// Result from command execution
#[derive(Debug)]
struct CommandOutput {
    _command: String,
    stdout: String,
    stderr: String,
    exit_code: i32,
}

/// Internal events for the command executor
#[derive(Debug)]
enum ExecutorEvent {
    Task(Task),
    CommandFinished {
        tool_call_id: String,
        result: Result<CommandOutput, String>,
    },
}

pub struct Command {
    executor_tx: Sender<Task>,
    config: ParsedConfig,
    pending_command: Option<PendingCommand>,
    executing_tool_calls: Vec<String>,
    worker_tx: Sender<Event>,
}

impl Command {
    pub fn new(worker_tx: Sender<Event>, config: ParsedConfig) -> Self {
        let (executor_tx, executor_rx) = unbounded();

        let worker_tx_clone = worker_tx.clone();
        // Start the executor thread
        thread::spawn(move || {
            execute_command_executor(worker_tx_clone, executor_rx);
        });

        Self {
            executor_tx,
            config,
            pending_command: None,
            executing_tool_calls: Vec::new(),
            worker_tx,
        }
    }

    pub fn has_pending_command(&self) -> bool {
        self.pending_command.is_some()
    }

    pub fn handle_user_confirmation(
        &mut self,
        input: &str,
    ) -> Option<(String, Vec<String>, String, bool)> {
        if let Some(pending) = self.pending_command.take() {
            let response = input.trim().to_lowercase();
            if response == "y" || response == "yes" {
                // Track executing command
                self.executing_tool_calls.push(pending.tool_call_id.clone());

                // Send internal log about user approval
                let _ = self.worker_tx.send(Event::CommandStageUpdate {
                    call_id: pending.tool_call_id.clone(),
                    stage: CommandExecutionStage::Executing {
                        command: format!("{} {}", pending.command, pending.args.join(" ")),
                    },
                });

                // Send command to executor
                if let Err(e) = self.executor_tx.send(Task::Execute {
                    command: pending.command.clone(),
                    args: pending.args.clone(),
                    tool_call_id: pending.tool_call_id.clone(),
                }) {
                    error!("Failed to send command to executor: {}", e);
                }
                None
            } else {
                // User denied - return info for denial response
                Some((pending.command, pending.args, pending.tool_call_id, false))
            }
        } else {
            None
        }
    }

    pub fn cancel_pending_operations(&mut self) {
        // Cancel any executing commands
        for tool_call_id in self.executing_tool_calls.drain(..) {
            let _ = self.executor_tx.send(Task::Cancel { tool_call_id });
        }

        // Clear any pending command
        self.pending_command = None;
    }

    pub fn name(&self) -> &'static str {
        TOOL_NAME
    }

    pub fn description(&self) -> &'static str {
        TOOL_DESCRIPTION
    }

    pub fn input_schema(&self) -> Value {
        serde_json::from_str(TOOL_INPUT_SCHEMA).unwrap()
    }

    pub fn handle_call(
        &mut self,
        tool_call: ToolCall,
        tui_tx: &Sender<tui::Task>,
    ) -> Result<Option<ToolResponse>, String> {
        // Parse the arguments
        let args = serde_json::from_value::<serde_json::Value>(tool_call.fn_arguments)
            .map_err(|e| format!("Failed to parse command arguments: {}", e))?;

        // Extract command and arguments
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing 'command' field in arguments".to_string())?;

        let args_array = match args.get("args") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<String>>(),
            _ => Vec::new(),
        };

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
            // Command is whitelisted, execute without prompting
            debug!(
                "Command '{}' is whitelisted, executing without prompt",
                command
            );

            // Send internal log stage for whitelisted command
            let _ = self.worker_tx.send(Event::CommandStageUpdate {
                call_id: tool_call.call_id.clone(),
                stage: CommandExecutionStage::Executing {
                    command: format!("{} {}", command, args_array.join(" ")),
                },
            });

            // Track executing command
            self.executing_tool_calls.push(tool_call.call_id.clone());

            // Send command directly to executor
            self.executor_tx
                .send(Task::Execute {
                    command: command.to_string(),
                    args: args_array,
                    tool_call_id: tool_call.call_id,
                })
                .map_err(|e| format!("Failed to send command to executor: {}", e))?;

            // No immediate response needed - results come through events
            Ok(None)
        } else {
            // Command not whitelisted, prompt for confirmation
            debug!(
                "Command '{}' is NOT whitelisted, prompting for confirmation",
                command
            );

            // Send command prompt stage update
            let _ = self.worker_tx.send(Event::CommandStageUpdate {
                call_id: tool_call.call_id.clone(),
                stage: CommandExecutionStage::AwaitingApproval {
                    command: command.to_string(),
                    args: args_array.clone(),
                },
            });

            let _ = tui_tx.send(tui::Task::AddEvent(
                tui::events::TuiEvent::set_waiting_for_confirmation(true),
            ));

            // Store the pending command
            self.pending_command = Some(PendingCommand {
                command: command.to_string(),
                args: args_array,
                tool_call_id: tool_call.call_id,
            });

            // No immediate response - waiting for user confirmation
            Ok(None)
        }
    }
}

/// Executes the command executor thread
fn execute_command_executor(tx: Sender<Event>, rx: Receiver<Task>) {
    if let Err(e) = do_execute_command_executor(tx, rx) {
        error!("Error while executing command executor: {e:?}");
    }
}

fn do_execute_command_executor(tx: Sender<Event>, rx: Receiver<Task>) -> CommandExecutorResult<()> {
    // Internal channel for handling command completions
    let (internal_tx, internal_rx) = unbounded::<ExecutorEvent>();

    // Track active cancellation tokens
    let cancellation_tokens: Arc<Mutex<HashMap<String, Sender<()>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Forward external tasks to internal channel
    let internal_tx_clone = internal_tx.clone();
    thread::spawn(move || {
        while let Ok(task) = rx.recv() {
            let _ = internal_tx_clone.send(ExecutorEvent::Task(task));
        }
    });

    // Main event loop
    while let Ok(event) = internal_rx.recv() {
        match event {
            ExecutorEvent::Task(task) => {
                match task {
                    Task::Execute {
                        command,
                        args,
                        tool_call_id,
                    } => {
                        // The executing stage is already sent when command is approved/whitelisted

                        let (cancel_tx, cancel_rx) = unbounded::<()>();

                        // Store cancellation token
                        {
                            let mut tokens = cancellation_tokens.lock().unwrap();
                            tokens.insert(tool_call_id.clone(), cancel_tx);
                        }

                        let internal_tx = internal_tx.clone();
                        let tool_call_id_clone = tool_call_id.clone();

                        thread::spawn(move || {
                            let result =
                                execute_command_with_cancellation(&command, &args, cancel_rx);

                            let _ = internal_tx.send(ExecutorEvent::CommandFinished {
                                tool_call_id: tool_call_id_clone,
                                result: result.map_err(|e| e.to_string()),
                            });
                        });
                    }
                    Task::Cancel { tool_call_id } => {
                        let mut tokens = cancellation_tokens.lock().unwrap();
                        if let Some(cancel_tx) = tokens.remove(&tool_call_id) {
                            // Send cancellation signal
                            let _ = cancel_tx.send(());
                        }
                    }
                }
            }
            ExecutorEvent::CommandFinished {
                tool_call_id,
                result,
            } => {
                // Remove cancellation token
                {
                    let mut tokens = cancellation_tokens.lock().unwrap();
                    tokens.remove(&tool_call_id);
                }

                // Send result to worker
                match result {
                    Ok(output) => {
                        let _ = tx.send(Event::CommandStageUpdate {
                            call_id: tool_call_id.clone(),
                            stage: CommandExecutionStage::Result {
                                stdout: output.stdout,
                                stderr: output.stderr,
                                exit_code: output.exit_code,
                            },
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(Event::CommandStageUpdate {
                            call_id: tool_call_id.clone(),
                            stage: CommandExecutionStage::Failed {
                                error: format!("Command execution failed: {}", e),
                            },
                        });
                    }
                }
            }
        }
    }

    Ok(())
}

/// Execute a command with cancellation support
fn execute_command_with_cancellation(
    command: &str,
    args: &[String],
    cancel_rx: Receiver<()>,
) -> CommandExecutorResult<CommandOutput> {
    // Spawn the command
    let child = StdCommand::new(command)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|_| ExecutionFailedSnafu {
            command: format!("{} {}", command, args.join(" ")),
        })?;

    // Check for cancellation in a separate thread
    let child_id = child.id();
    thread::spawn(move || {
        if let Ok(()) = cancel_rx.recv() {
            // Try to kill the process using its ID
            #[cfg(unix)]
            unsafe {
                libc::kill(child_id as i32, libc::SIGTERM);
            }
            #[cfg(windows)]
            {
                // On Windows, we'd use TerminateProcess, but for now just log
                error!("Process cancellation not fully implemented on Windows");
            }
        }
    });

    // Wait for the command to complete
    let output = child
        .wait_with_output()
        .with_context(|_| ExecutionFailedSnafu {
            command: format!("{} {}", command, args.join(" ")),
        })?;

    // Check if we were cancelled
    if !output.status.success() && output.status.code().is_none() {
        return Err(CommandExecutorError::Cancelled);
    }

    // Convert output to string
    let stdout = String::from_utf8(output.stdout).context(OutputFailedSnafu)?;
    let stderr = String::from_utf8(output.stderr).context(OutputFailedSnafu)?;

    Ok(CommandOutput {
        _command: format!("{} {}", command, args.join(" ")),
        stdout,
        stderr,
        exit_code: output.status.code().unwrap_or(-1),
    })
}


use std::collections::HashMap;
use std::process::{Command as StdCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use crossbeam::channel::{Receiver, Sender, unbounded};
use genai::chat::ToolCall;
use serde_json::{json, Value};
use snafu::{ResultExt, Snafu};
use tracing::{debug, error};

use crate::{config::ParsedConfig, tui, worker::Event};

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

/// Tasks the command executor can receive from the worker
#[derive(Debug, Clone)]
pub enum Task {
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
    command: String,
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

pub struct Command {}

impl Command {
    pub fn new() -> Self {
        Command {}
    }
}

impl crate::tools::InternalTool for Command {
    fn name(&self) -> &'static str {
        "execute_command"
    }

    fn description(&self) -> &'static str {
        "Execute a shell command with specified arguments. E.G. pwd, git, ls, etc..."
    }

    fn input_schema(&self) -> Value {
        json!({
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
        })
    }
}


/// Handle the execute_command tool (legacy function for worker.rs)
pub fn handle_execute_command(
    tool_call: ToolCall,
    pending_command: &mut Option<PendingCommand>,
    tui_tx: &Sender<tui::Task>,
    config: &ParsedConfig,
    command_executor_tx: &Sender<Task>,
    _worker_tx: &Sender<Event>,
    executing_tool_calls: &mut Vec<String>,
) {
    // Parse the arguments
    let args = match serde_json::from_value::<serde_json::Value>(tool_call.fn_arguments) {
        Ok(args) => args,
        Err(e) => {
            error!("Failed to parse command arguments: {}", e);
            return;
        }
    };

    // Extract command and arguments
    let command = match args.get("command").and_then(|v| v.as_str()) {
        Some(cmd) => cmd,
        None => {
            error!("Missing 'command' field in arguments");
            return;
        }
    };

    let args_array = match args.get("args") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect::<Vec<String>>(),
        _ => Vec::new(),
    };

    // Check if command is whitelisted
    debug!("Checking if command '{}' is whitelisted", command);
    debug!("Whitelisted commands: {:?}", config.whitelisted_commands);

    // Check for exact match or if command starts with a whitelisted command
    // This handles cases like "git status" where "git" is whitelisted
    let is_whitelisted = config.whitelisted_commands.iter().any(|wc| {
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
        let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::system(format!(
            "Executing whitelisted command: {} {}",
            command,
            args_array.join(" ")
        ))));

        // Track executing command
        executing_tool_calls.push(tool_call.call_id.clone());

        // Send command directly to executor
        let _ = command_executor_tx.send(Task::Execute {
            command: command.to_string(),
            args: args_array,
            tool_call_id: tool_call.call_id,
        });
    } else {
        // Command not whitelisted, prompt for confirmation
        debug!(
            "Command '{}' is NOT whitelisted, prompting for confirmation",
            command
        );
        let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::command_prompt(
            command.to_string(),
            args_array.clone(),
        )));
        let _ = tui_tx.send(tui::Task::AddEvent(
            tui::events::TuiEvent::set_waiting_for_confirmation(true),
        ));

        // Store the pending command
        *pending_command = Some(PendingCommand {
            command: command.to_string(),
            args: args_array,
            tool_call_id: tool_call.call_id,
        });
    }
}

/// Executes the command executor thread
pub fn execute_command_executor(
    tx: Sender<Event>, 
    rx: Receiver<Task>,
    config: ParsedConfig
) {
    if let Err(e) = do_execute_command_executor(tx, rx, config) {
        error!("Error while executing command executor: {e:?}");
    }
}

fn do_execute_command_executor(
    tx: Sender<Event>,
    rx: Receiver<Task>,
    _config: ParsedConfig,
) -> CommandExecutorResult<()> {
    // Internal channel for handling command completions
    let (internal_tx, internal_rx) = unbounded::<ExecutorEvent>();
    
    // Track active cancellation tokens
    let cancellation_tokens: Arc<Mutex<HashMap<String, Sender<()>>>> = Arc::new(Mutex::new(HashMap::new()));

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
                    Task::Execute { command, args, tool_call_id } => {
                        let (cancel_tx, cancel_rx) = unbounded::<()>();
                        
                        // Store cancellation token
                        {
                            let mut tokens = cancellation_tokens.lock().unwrap();
                            tokens.insert(tool_call_id.clone(), cancel_tx);
                        }
                        
                        let internal_tx = internal_tx.clone();
                        let tool_call_id_clone = tool_call_id.clone();
                        
                        thread::spawn(move || {
                            let result = execute_command_with_cancellation(
                                &command, 
                                &args, 
                                cancel_rx
                            );
                            
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
            ExecutorEvent::CommandFinished { tool_call_id, result } => {
                // Remove cancellation token
                {
                    let mut tokens = cancellation_tokens.lock().unwrap();
                    tokens.remove(&tool_call_id);
                }
                
                // Send result to worker
                match result {
                    Ok(output) => {
                        let _ = tx.send(Event::CommandExecutionResult {
                            tool_call_id,
                            command: output.command,
                            stdout: output.stdout,
                            stderr: output.stderr,
                            exit_code: output.exit_code,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(Event::CommandExecutionResult {
                            tool_call_id,
                            command: String::new(),
                            stdout: String::new(),
                            stderr: format!("Command execution failed: {}", e),
                            exit_code: -1,
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
    let output = child.wait_with_output().with_context(|_| ExecutionFailedSnafu {
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
        command: format!("{} {}", command, args.join(" ")),
        stdout,
        stderr,
        exit_code: output.status.code().unwrap_or(-1),
    })
}
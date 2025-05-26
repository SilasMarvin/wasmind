use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use crossbeam::channel::{Receiver, Sender, unbounded};
use snafu::{ResultExt, Snafu};
use tracing::error;

use crate::{config::ParsedConfig, worker};

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

    #[snafu(display("Failed to send event to worker"))]
    SendEvent {
        #[snafu(source)]
        source: crossbeam::channel::SendError<worker::Event>,
    },

    #[snafu(display("Command was cancelled"))]
    Cancelled,
}

type CommandExecutorResult<T> = Result<T, CommandExecutorError>;

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

/// Internal events for the command executor
#[derive(Debug)]
enum ExecutorEvent {
    Task(Task),
    CommandFinished {
        tool_call_id: String,
        result: Result<String, String>,
    },
}

/// Executes the command executor thread
pub fn execute_command_executor(
    tx: Sender<worker::Event>, 
    rx: Receiver<Task>,
    config: ParsedConfig
) {
    if let Err(e) = do_execute_command_executor(tx, rx, config) {
        error!("Error while executing command executor: {e:?}");
    }
}

fn do_execute_command_executor(
    tx: Sender<worker::Event>,
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
                let output = match result {
                    Ok(output) => output,
                    Err(e) => format!("Command execution failed: {}", e),
                };
                
                let _ = tx.send(worker::Event::CommandExecutionResult(tool_call_id, output));
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
) -> CommandExecutorResult<String> {
    println!("\n[EXECUTING COMMAND]");
    println!("  $ {} {}", command, args.join(" "));

    // Spawn the command
    let child = Command::new(command)
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

    // Display output to user
    if !stdout.is_empty() {
        println!("\n[STDOUT]");
        println!("{}", stdout);
    }

    if !stderr.is_empty() {
        println!("\n[STDERR]");
        println!("{}", stderr);
    }

    println!(
        "\n[COMMAND COMPLETED] Exit code: {}",
        output.status.code().unwrap_or(-1)
    );

    // Return combined output
    let mut result = String::new();
    if !stdout.is_empty() {
        result.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !result.is_empty() {
            result.push_str("\n\nSTDERR:\n");
        }
        result.push_str(&stderr);
    }

    if result.is_empty() {
        result = format!(
            "Command completed with exit code: {}",
            output.status.code().unwrap_or(-1)
        );
    }

    Ok(result)
}
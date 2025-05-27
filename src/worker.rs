use base64::{Engine, engine::general_purpose::STANDARD};
use crossbeam::channel::{Receiver, Sender, unbounded};
use genai::chat::{
    ChatMessage, ChatRequest, ChatRole, ContentPart, MessageContent, Tool,
    ToolCall, ToolResponse,
};
use image::ImageFormat;
use serde_json;
use snafu::ResultExt;
use std::io::Cursor;
use tracing::error;

use crate::{
    AssistantTaskSendSnafu, MCPTaskSendSnafu, MicrophoneTaskSendSnafu, SResult, assistant,
    config::ParsedConfig,
    context::{clipboard::capture_clipboard, microphone, screen::capture_screen},
    mcp,
    tools::{command::PendingCommand, command_executor, planner},
    tui,
};

/// Task status for the planner
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Skipped,
}

/// Individual task in the plan
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Task {
    pub description: String,
    pub status: TaskStatus,
}

/// Task plan managed by the planner tool
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskPlan {
    pub title: String,
    pub tasks: Vec<Task>,
}

/// All available events the worker can handle
#[derive(Debug)]
pub enum Event {
    UserTUIInput(String),
    MCPToolsInit(Vec<Tool>),
    MCPToolsResponse(Vec<ToolResponse>),
    Action(Action),
    ChatResponse(MessageContent),
    MicrophoneResponse(String),
    CommandExecutionResult {
        tool_call_id: String,
        command: String,
        stdout: String,
        stderr: String,
        exit_code: i32,
    },
}

/// Actions the worker can perform and users can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    CaptureWindow,
    CaptureClipboard,
    ToggleRecordMicrophone,
    Assist,
    CancelAssist,
    Exit,
}

impl Action {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "CaptureWindow" => Some(Action::CaptureWindow),
            "CaptureClipboard" => Some(Action::CaptureClipboard),
            "ToggleRecordMicrophone" => Some(Action::ToggleRecordMicrophone),
            "Assist" => Some(Action::Assist),
            "CancelAssist" => Some(Action::CancelAssist),
            "Exit" => Some(Action::Exit),
            _ => None,
        }
    }
}

pub fn execute_worker(tx: Sender<Event>, rx: Receiver<Event>, config: ParsedConfig) {
    // Start TUI in a separate thread
    let (tui_tx, tui_rx) = unbounded();
    let worker_tx_clone = tx.clone();
    let config_clone = config.clone();

    let tui_handle = std::thread::spawn(move || {
        if let Err(e) = tui::execute_tui(worker_tx_clone, tui_rx, config_clone) {
            error!("Error executing TUI: {e:?}");
        }
    });

    if let Err(e) = do_execute_worker(tx, rx, config, tui_tx.clone()) {
        error!("Error executing worker: {e:?}");
        let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::error(format!(
            "Error executing worker: {e:?}"
        ))));
    }

    // Signal TUI to exit
    let _ = tui_tx.send(tui::Task::Exit);

    // Wait for TUI to finish
    let _ = tui_handle.join();
}

pub fn do_execute_worker(
    tx: Sender<Event>,
    rx: Receiver<Event>,
    config: ParsedConfig,
    tui_tx: Sender<tui::Task>,
) -> SResult<()> {
    let mut chat_request = ChatRequest::default().with_system(&config.model.system_prompt);
    let mut parts = vec![];

    // Track pending command execution
    let mut pending_command: Option<PendingCommand> = None;
    let mut executing_tool_calls: Vec<String> = Vec::new();
    
    // Track current task plan
    let mut current_task_plan: Option<TaskPlan> = None;

    let (assistant_tx, assistant_rx) = unbounded();
    let local_worker_tx = tx.clone();
    let local_config = config.clone();
    let _assistant_handle = std::thread::spawn(move || {
        assistant::execute_assistant(local_worker_tx, assistant_rx, local_config);
    });

    let (mcp_tx, mcp_rx) = unbounded();
    let local_mcp_tx = tx.clone();
    let local_config = config.clone();
    let _mcp_handle = std::thread::spawn(move || {
        crate::mcp::execute_mcp(local_mcp_tx, mcp_rx, local_config);
    });

    let (microphone_tx, microphone_rx) = unbounded();
    let local_microphone_tx = tx.clone();
    let local_config = config.clone();
    let _audio_handle = std::thread::spawn(move || {
        microphone::execute_microphone(local_microphone_tx, microphone_rx, local_config);
    });

    let (command_executor_tx, command_executor_rx) = unbounded();
    let local_command_executor_tx = tx.clone();
    let local_config = config.clone();
    let _command_executor_handle = std::thread::spawn(move || {
        command_executor::execute_command_executor(
            local_command_executor_tx,
            command_executor_rx,
            local_config,
        );
    });

    let mut waiting_for_assistant_response = false;
    let mut microphone_recording = false;

    while let Ok(task) = rx.recv() {
        match task {
            Event::MCPToolsInit(mut tools) => {
                // Add internal tools
                tools.push(Tool {
                    name: "execute_command".to_string(),
                    description: Some("Execute a command line command with user confirmation. Use this to print the current working directory, run commands like git status, npm install, cargo build, etc. The user will be prompted to approve the command before execution.".to_string()),
                    schema: Some(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "command": {
                                "type": "string",
                                "description": "The command to execute (e.g., 'ls', 'git', 'npm')"
                            },
                            "args": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "description": "Array of arguments to pass to the command"
                            }
                        },
                        "required": ["command"]
                    })),
                });
                
                tools.push(Tool {
                    name: "planner".to_string(),
                    description: Some("Create and manage a task plan to break down complex tasks into numbered steps. Use this to organize your work, track progress, and update the plan as you complete tasks.".to_string()),
                    schema: Some(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "action": {
                                "type": "string",
                                "enum": ["create", "update", "complete", "start", "skip"],
                                "description": "Action to perform: create (new plan), update (modify task), complete (mark done), start (mark in progress), skip (mark skipped)"
                            },
                            "title": {
                                "type": "string",
                                "description": "Title of the task plan (required for create action)"
                            },
                            "tasks": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "description": "List of task descriptions (required for create action)"
                            },
                            "task_number": {
                                "type": "number",
                                "description": "Task number to update (1-based, required for update/complete/start/skip actions)"
                            },
                            "new_description": {
                                "type": "string",
                                "description": "New description for the task (optional, for update action)"
                            }
                        },
                        "required": ["action"]
                    })),
                });

                chat_request = chat_request.with_tools(tools);
            }
            Event::MicrophoneResponse(text) => {
                microphone_recording = false;
                parts.push(ContentPart::from_text(text.clone()));
                // Add to TUI
                let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::microphone_stopped()));
                let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::user_microphone(
                    text,
                )));
                // This is kind of silly but rust ownership is being annoying
                tx.send(Event::Action(Action::Assist))
                    .whatever_context("Error sending assist event to worker from worker")?;
            }
            Event::UserTUIInput(text) => {
                // Check if we're waiting for command confirmation
                if let Some(pending) = pending_command.take() {
                    let response = text.trim().to_lowercase();
                    if response == "y" || response == "yes" {
                        // Track executing command
                        executing_tool_calls.push(pending.tool_call_id.clone());
                        // Send command to executor
                        command_executor_tx
                            .send(command_executor::Task::Execute {
                                command: pending.command,
                                args: pending.args,
                                tool_call_id: pending.tool_call_id,
                            })
                            .whatever_context("Error sending command to executor")?;
                    } else {
                        // User denied
                        tx.send(Event::CommandExecutionResult {
                            tool_call_id: pending.tool_call_id,
                            command: format!("{} {}", pending.command, pending.args.join(" ")),
                            stdout: String::new(),
                            stderr: "Command execution denied by user".to_string(),
                            exit_code: -1,
                        })
                        .whatever_context("Error sending command denial")?;
                    }
                    let _ = tui_tx.send(tui::Task::ClearInput);
                    let _ = tui_tx.send(tui::Task::AddEvent(
                        tui::events::TuiEvent::set_waiting_for_confirmation(false),
                    ));
                    continue; // Don't process this as regular input
                }

                parts.push(ContentPart::from_text(text.clone()));
                // Add to TUI
                let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::user_input(text)));
                let _ = tui_tx.send(tui::Task::ClearInput);
                // This is kind of silly but rust ownership is being annoying
                tx.send(Event::Action(Action::Assist))
                    .whatever_context("Error sending assist event to worker from worker")?;
            }
            Event::Action(action) => match action {
                Action::ToggleRecordMicrophone => {
                    microphone_tx
                        .send(microphone::Task::ToggleRecord)
                        .context(MicrophoneTaskSendSnafu)?;
                    microphone_recording = !microphone_recording;
                    if microphone_recording {
                        let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::microphone_started()));
                    }
                }
                Action::CaptureWindow => {
                    let image = capture_screen()?;
                    let mut buffer = Cursor::new(Vec::new());
                    image.write_to(&mut buffer, ImageFormat::Png).unwrap();
                    let base64 = STANDARD.encode(buffer.into_inner());
                    parts.push(ContentPart::from_image_base64("image/png", base64.clone()));
                    let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::screenshot(
                        "Screenshot captured".to_string(),
                    )));
                }
                Action::CaptureClipboard => {
                    let text = capture_clipboard()?;
                    parts.push(ContentPart::from_text(text.clone()));
                    let _ =
                        tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::clipboard(text)));
                }
                Action::Assist => {
                    if waiting_for_assistant_response {
                        continue;
                    }
                    chat_request = chat_request
                        .append_message(ChatMessage::user(MessageContent::Parts(parts)));
                    assistant_tx
                        .send(assistant::Task::Assist(chat_request.clone()))
                        .context(AssistantTaskSendSnafu)?;
                    parts = vec![];
                    waiting_for_assistant_response = true;
                    let _ = tui_tx.send(tui::Task::AddEvent(
                        tui::events::TuiEvent::set_waiting_for_response(true),
                    ));
                }
                Action::CancelAssist => {
                    if waiting_for_assistant_response {
                        assistant_tx
                            .send(assistant::Task::Cancel)
                            .context(AssistantTaskSendSnafu)?;
                        waiting_for_assistant_response = false;

                        // Cancel any executing commands
                        for tool_call_id in executing_tool_calls.drain(..) {
                            command_executor_tx
                                .send(command_executor::Task::Cancel { tool_call_id })
                                .whatever_context("Error sending cancel to command executor")?;
                        }

                        // Clear any pending command
                        pending_command = None;

                        let _ = tui_tx.send(tui::Task::AddEvent(
                            tui::events::TuiEvent::set_waiting_for_response(false),
                        ));
                        let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::system(
                            "Cancelled assistant response".to_string(),
                        )));
                    }
                }
                Action::Exit => {
                    let _ = tui_tx.send(tui::Task::Exit);
                    break;
                }
            },
            Event::MCPToolsResponse(call_tool_results) => {
                chat_request = chat_request.append_message(ChatMessage {
                    role: ChatRole::Tool,
                    content: MessageContent::ToolResponses(call_tool_results),
                    options: None,
                });
                assistant_tx
                    .send(assistant::Task::Assist(chat_request.clone()))
                    .context(AssistantTaskSendSnafu)?;
            }
            Event::ChatResponse(message_content) => {
                match message_content.clone() {
                    MessageContent::Text(text) => {
                        let _ = tui_tx.send(tui::Task::AddEvent(
                            tui::events::TuiEvent::assistant_response(text, false),
                        ));
                        waiting_for_assistant_response = false;
                        let _ = tui_tx.send(tui::Task::AddEvent(
                            tui::events::TuiEvent::set_waiting_for_response(false),
                        ));
                    }
                    MessageContent::Parts(content_parts) => {
                        for part in content_parts {
                            match part {
                                ContentPart::Text(text) => {
                                    let _ = tui_tx.send(tui::Task::AddEvent(
                                        tui::events::TuiEvent::assistant_response(text, false),
                                    ));
                                }
                                ContentPart::Image {
                                    content_type: _,
                                    source: _,
                                } => todo!(),
                            }
                        }
                        waiting_for_assistant_response = false;
                        let _ = tui_tx.send(tui::Task::AddEvent(
                            tui::events::TuiEvent::set_waiting_for_response(false),
                        ));
                    }
                    MessageContent::ToolCalls(tool_calls) => {
                        // Display function calls
                        for call in &tool_calls {
                            let _ = tui_tx.send(tui::Task::AddEvent(
                                tui::events::TuiEvent::function_call(
                                    call.fn_name.clone(),
                                    Some(call.fn_arguments.to_string()),
                                ),
                            ));
                        }

                        // Separate internal tools from MCP tools
                        let (internal_tools, mcp_tools): (Vec<_>, Vec<_>) = tool_calls
                            .into_iter()
                            .partition(|tool_call| is_internal_tool(&tool_call.fn_name));

                        // Handle internal tools
                        if !internal_tools.is_empty() {
                            handle_internal_tools(internal_tools, &mut pending_command, &mut current_task_plan, &tui_tx, &tx, &config, &command_executor_tx, &mut executing_tool_calls);
                        }

                        // Send remaining tools to MCP
                        if !mcp_tools.is_empty() {
                            mcp_tx
                                .send(mcp::Task::UseTools(mcp_tools))
                                .context(MCPTaskSendSnafu)?;
                        }
                    }
                    // Right now we don't expect tool responses from the assistant
                    MessageContent::ToolResponses(_tool_responses) => unreachable!(),
                }

                chat_request = chat_request.append_message(ChatMessage {
                    role: ChatRole::Assistant,
                    content: message_content,
                    options: None,
                });
            }
            Event::CommandExecutionResult {
                tool_call_id,
                command,
                stdout,
                stderr,
                exit_code,
            } => {
                // Remove from executing list
                executing_tool_calls.retain(|id| id != &tool_call_id);

                // Add command result to TUI
                let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::command_result(
                    command.clone(),
                    stdout.clone(),
                    stderr.clone(),
                    exit_code,
                )));

                // Format the result for the LLM
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
                    result = format!("Command completed with exit code: {}", exit_code);
                }

                // Send the command result as a tool response
                let tool_response = ToolResponse {
                    call_id: tool_call_id,
                    content: result,
                };
                tx.send(Event::MCPToolsResponse(vec![tool_response]))
                    .whatever_context("Error sending command execution result")?;
            }
        }
    }

    Ok(())
}

/// Check if a tool is an internal tool
fn is_internal_tool(tool_name: &str) -> bool {
    matches!(tool_name, "execute_command" | "planner")
}

/// Handle internal tool calls
fn handle_internal_tools(
    tool_calls: Vec<ToolCall>,
    pending_command: &mut Option<PendingCommand>,
    current_task_plan: &mut Option<TaskPlan>,
    tui_tx: &Sender<tui::Task>,
    worker_tx: &Sender<Event>,
    config: &ParsedConfig,
    command_executor_tx: &Sender<command_executor::Task>,
    executing_tool_calls: &mut Vec<String>,
) {
    for tool_call in tool_calls {
        match tool_call.fn_name.as_str() {
            "execute_command" => handle_execute_command(tool_call, pending_command, tui_tx, config, command_executor_tx, worker_tx, executing_tool_calls),
            "planner" => planner::handle_planner(tool_call, current_task_plan, tui_tx, worker_tx),
            _ => {
                // For unknown tools, we should send an error response
                // but since we're not sending responses anymore, we'll just log it
                error!("Unknown internal tool: {}", tool_call.fn_name);
            }
        }
    }
}

/// Handle the execute_command tool
fn handle_execute_command(
    tool_call: ToolCall,
    pending_command: &mut Option<PendingCommand>,
    tui_tx: &Sender<tui::Task>,
    config: &ParsedConfig,
    command_executor_tx: &Sender<command_executor::Task>,
    worker_tx: &Sender<Event>,
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
    tracing::debug!("Checking if command '{}' is whitelisted", command);
    tracing::debug!("Whitelisted commands: {:?}", config.whitelisted_commands);
    
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
        tracing::debug!("Command '{}' is whitelisted, executing without prompt", command);
        let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::system(
            format!("Executing whitelisted command: {} {}", command, args_array.join(" "))
        )));
        
        // Track executing command
        executing_tool_calls.push(tool_call.call_id.clone());
        
        // Send command directly to executor
        let _ = command_executor_tx.send(command_executor::Task::Execute {
            command: command.to_string(),
            args: args_array,
            tool_call_id: tool_call.call_id,
        });
    } else {
        // Command not whitelisted, prompt for confirmation
        tracing::debug!("Command '{}' is NOT whitelisted, prompting for confirmation", command);
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

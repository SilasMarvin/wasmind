use base64::{Engine, engine::general_purpose::STANDARD};
use crossbeam::channel::{Receiver, Sender, unbounded};
use genai::chat::{
    ChatMessage, ChatRequest, ChatRole, ChatStreamEvent, ContentPart, MessageContent, Tool,
    ToolResponse, ToolCall,
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
    mcp, tui,
    tools::{
        command::{display_command_prompt, PendingCommand},
        command_executor,
    },
};

/// All available events the worker can handle
#[derive(Debug)]
pub enum Event {
    UserTUIInput(String),
    MCPToolsInit(Vec<Tool>),
    MCPToolsResponse(Vec<ToolResponse>),
    Action(Action),
    ChatStreamEvent(ChatStreamEvent),
    ChatResponse(MessageContent),
    MicrophoneResponse(String),
    CommandExecutionResult(String, String), // tool_call_id, result
}

/// Actions the worker can perform and users can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    CaptureWindow,
    CaptureClipboard,
    ToggleRecordMicrophone,
    Assist,
    CancelAssist,
}

impl Action {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "CaptureWindow" => Some(Action::CaptureWindow),
            "CaptureClipboard" => Some(Action::CaptureClipboard),
            "ToggleRecordMicrophone" => Some(Action::ToggleRecordMicrophone),
            "Assist" => Some(Action::Assist),
            "CancelAssist" => Some(Action::CancelAssist),
            _ => None,
        }
    }
}

pub fn execute_worker(tx: Sender<Event>, rx: Receiver<Event>, config: ParsedConfig) {
    if let Err(e) = do_execute_worker(tx, rx, config) {
        error!("Error executing worker: {e:?}");
        tui::display_error(&format!("Error executing worker: {e:?}"));
    }
}

pub fn do_execute_worker(
    tx: Sender<Event>,
    rx: Receiver<Event>,
    config: ParsedConfig,
) -> SResult<()> {
    let mut chat_request = ChatRequest::default().with_system(&config.model.system_prompt);
    let mut parts = vec![];
    
    // Track pending command execution
    let mut pending_command: Option<PendingCommand> = None;
    let mut executing_tool_calls: Vec<String> = Vec::new();

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
        command_executor::execute_command_executor(local_command_executor_tx, command_executor_rx, local_config);
    });

    let mut waiting_for_assistant_response = false;

    tui::display_user_prompt();
    while let Ok(task) = rx.recv() {
        match task {
            Event::MCPToolsInit(mut tools) => {
                // Add internal tools
                tools.push(Tool {
                    name: "execute_command".to_string(),
                    description: Some("Execute a command line command with user confirmation. The user will be prompted to approve the command before execution.".to_string()),
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
                
                chat_request = chat_request.with_tools(tools);
            }
            Event::MicrophoneResponse(text) => {
                parts.push(ContentPart::from_text(text.clone()));
                // This is kind of silly but rust ownership is being annoying
                tx.send(Event::Action(Action::Assist))
                    .whatever_context("Error sending assist event to worker from worker")?;
                tui::display_user_microphone_input(&text);
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
                        tx.send(Event::CommandExecutionResult(
                            pending.tool_call_id,
                            "Command execution denied by user".to_string()
                        ))
                        .whatever_context("Error sending command denial")?;
                    }
                    continue; // Don't process this as regular input
                }
                
                parts.push(ContentPart::from_text(text));
                // This is kind of silly but rust ownership is being annoying
                tx.send(Event::Action(Action::Assist))
                    .whatever_context("Error sending assist event to worker from worker")?;
            }
            Event::Action(action) => match action {
                Action::ToggleRecordMicrophone => {
                    microphone_tx
                        .send(microphone::Task::ToggleRecord)
                        .context(MicrophoneTaskSendSnafu)?;
                }
                Action::CaptureWindow => {
                    let image = capture_screen()?;
                    let mut buffer = Cursor::new(Vec::new());
                    image.write_to(&mut buffer, ImageFormat::Png).unwrap();
                    let base64 = STANDARD.encode(buffer.into_inner());
                    parts.push(ContentPart::from_image_base64("image/png", base64.clone()));
                    tui::display_screenshot(&format!("Screenshot_FILLER",));
                }
                Action::CaptureClipboard => {
                    let text = capture_clipboard()?;
                    parts.push(ContentPart::from_text(text.clone()));
                    tui::display_clipboard_excerpt(&text);
                }
                Action::Assist => {
                    if waiting_for_assistant_response {
                        continue;
                    }
                    tui::display_done_marker();
                    chat_request = chat_request
                        .append_message(ChatMessage::user(MessageContent::Parts(parts)));
                    assistant_tx
                        .send(assistant::Task::Assist(chat_request.clone()))
                        .context(AssistantTaskSendSnafu)?;
                    parts = vec![];
                    waiting_for_assistant_response = true;
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
                        
                        tui::display_user_prompt();
                    }
                }
            },
            Event::ChatStreamEvent(event) => unreachable!(),
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
                        tui::display_assistant_start();
                        tui::display_assistant_response(&text);
                        tui::display_user_prompt();
                        waiting_for_assistant_response = false;
                    }
                    MessageContent::Parts(content_parts) => {
                        for part in content_parts {
                            match part {
                                ContentPart::Text(text) => {
                                    tui::display_assistant_response_part(&text)
                                }
                                ContentPart::Image {
                                    content_type,
                                    source,
                                } => todo!(),
                            }
                        }
                        tui::display_user_prompt();
                        waiting_for_assistant_response = false;
                    }
                    MessageContent::ToolCalls(tool_calls) => {
                        tui::display_func_calls(
                            tool_calls
                                .iter()
                                .map(|tool_call| tool_call.fn_name.as_str())
                                .collect::<Vec<&str>>(),
                        );
                        
                        // Separate internal tools from MCP tools
                        let (internal_tools, mcp_tools): (Vec<_>, Vec<_>) = tool_calls
                            .into_iter()
                            .partition(|tool_call| is_internal_tool(&tool_call.fn_name));
                        
                        // Handle internal tools
                        if !internal_tools.is_empty() {
                            handle_internal_tools(internal_tools, &mut pending_command);
                        }
                        
                        // Send remaining tools to MCP
                        if !mcp_tools.is_empty() {
                            mcp_tx
                                .send(mcp::Task::UseTools(mcp_tools))
                                .context(MCPTaskSendSnafu)?;
                        }
                    }
                    // Right now we don't expect tool responses from the assistant
                    MessageContent::ToolResponses(tool_responses) => unreachable!(),
                }

                chat_request = chat_request.append_message(ChatMessage {
                    role: ChatRole::Assistant,
                    content: message_content,
                    options: None,
                });
            }
            Event::CommandExecutionResult(tool_call_id, result) => {
                // Remove from executing list
                executing_tool_calls.retain(|id| id != &tool_call_id);
                
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
    matches!(tool_name, "execute_command")
}

/// Handle internal tool calls
fn handle_internal_tools(tool_calls: Vec<ToolCall>, pending_command: &mut Option<PendingCommand>) {
    for tool_call in tool_calls {
        match tool_call.fn_name.as_str() {
            "execute_command" => handle_execute_command(tool_call, pending_command),
            _ => {
                // For unknown tools, we should send an error response
                // but since we're not sending responses anymore, we'll just log it
                error!("Unknown internal tool: {}", tool_call.fn_name);
            },
        }
    }
}

/// Handle the execute_command tool
fn handle_execute_command(tool_call: ToolCall, pending_command: &mut Option<PendingCommand>) {
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
        Some(serde_json::Value::Array(arr)) => {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<String>>()
        }
        _ => Vec::new(),
    };
    
    // Display the confirmation prompt
    display_command_prompt(command, &args_array);
    
    // Store the pending command
    *pending_command = Some(PendingCommand {
        command: command.to_string(),
        args: args_array,
        tool_call_id: tool_call.call_id,
    });
}

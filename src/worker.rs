use base64::{Engine, engine::general_purpose::STANDARD};
use crossbeam::channel::{Receiver, Sender, unbounded};
use genai::chat::{
    ChatMessage, ChatRequest, ChatRole, ContentPart, MessageContent, Tool, ToolResponse,
};
use image::ImageFormat;
use snafu::ResultExt;
use std::io::Cursor;
use tracing::error;

use crate::{
    AssistantTaskSendSnafu, MCPTaskSendSnafu, MicrophoneTaskSendSnafu, SResult, assistant,
    config::ParsedConfig,
    context::{clipboard::capture_clipboard, microphone, screen::capture_screen},
    mcp,
    system_state::SystemState,
    template::ToolInfo,
    tools::InternalToolHandler,
    tui,
};

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
    // We'll initialize the system prompt later when we have all tools available
    let mut chat_request = ChatRequest::default();
    let mut parts = vec![];

    // Create the system state for tracking files and plans
    let mut system_state = SystemState::new();

    // Create the internal tool handler
    let mut tool_handler = InternalToolHandler::new(tx.clone(), tui_tx.clone(), config.clone());

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

    let mut waiting_for_assistant_response = false;
    let mut microphone_recording = false;

    while let Ok(task) = rx.recv() {
        match task {
            Event::MCPToolsInit(mut tools) => {
                // Add internal tools to the list so the assistant knows about them
                for tool_name in tool_handler.get_tool_names() {
                    if let Some((name, description, schema)) =
                        tool_handler.get_tool_info(&tool_name)
                    {
                        tools.push(Tool {
                            name: name.to_string(),
                            description: Some(description.to_string()),
                            schema: Some(schema),
                        });
                    }
                }

                // Now render the system prompt with all available tools
                // Build tool infos
                let tool_infos: Vec<ToolInfo> = tools
                    .iter()
                    .filter_map(|tool| {
                        tool.description.as_ref().map(|desc| ToolInfo {
                            name: tool.name.clone(),
                            description: desc.clone(),
                        })
                    })
                    .collect();

                let rendered_prompt = system_state
                    .render_system_prompt(
                        &config.model.system_prompt,
                        &tool_infos,
                        config.whitelisted_commands.clone(),
                    )
                    .whatever_context("Failed to render system prompt template")?;
                chat_request = chat_request.with_system(&rendered_prompt);
                system_state.reset_modified();

                chat_request = chat_request.with_tools(tools);
            }
            Event::MicrophoneResponse(text) => {
                microphone_recording = false;
                parts.push(ContentPart::from_text(text.clone()));
                // Add to TUI
                let _ = tui_tx.send(tui::Task::AddEvent(
                    tui::events::TuiEvent::microphone_stopped(),
                ));
                let _ = tui_tx.send(tui::Task::AddEvent(tui::events::TuiEvent::user_microphone(
                    text,
                )));
                // This is kind of silly but rust ownership is being annoying
                tx.send(Event::Action(Action::Assist))
                    .whatever_context("Error sending assist event to worker from worker")?;
            }
            Event::UserTUIInput(text) => {
                // Let the tool handler process user input first
                match tool_handler.handle_user_input(&text) {
                    Ok(consumed) => {
                        if consumed {
                            // Input was consumed by a tool, don't process as chat
                            continue;
                        }
                    }
                    Err(e) => {
                        error!("Error handling user input in tools: {}", e);
                    }
                }

                // Process as chat input
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
                        let _ = tui_tx.send(tui::Task::AddEvent(
                            tui::events::TuiEvent::microphone_started(),
                        ));
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

                        // Cancel any pending tool operations
                        tool_handler.cancel_pending_operations();

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
                        let (internal_tools, mcp_tools): (Vec<_>, Vec<_>) =
                            tool_calls.into_iter().partition(|tool_call| {
                                tool_handler.is_internal_tool(&tool_call.fn_name)
                            });

                        // Handle internal tools
                        if !internal_tools.is_empty() {
                            let responses =
                                tool_handler.handle_tool_calls(internal_tools, &mut system_state);
                            if !responses.is_empty() {
                                tx.send(Event::MCPToolsResponse(responses))
                                    .whatever_context("Error sending tool responses")?;
                            }

                            // Re-render system prompt if state changed
                            if system_state.is_modified() {
                                let tool_infos: Vec<ToolInfo> = chat_request
                                    .tools
                                    .as_ref()
                                    .map_or(&vec![], |v| v)
                                    .iter()
                                    .filter_map(|tool| {
                                        tool.description.as_ref().map(|desc| ToolInfo {
                                            name: tool.name.clone(),
                                            description: desc.clone(),
                                        })
                                    })
                                    .collect();

                                let rendered = system_state
                                    .render_system_prompt(
                                        &config.model.system_prompt,
                                        &tool_infos,
                                        config.whitelisted_commands.clone(),
                                    )
                                    .whatever_context(
                                        "Failed to re-render system prompt template",
                                    )?;

                                chat_request = chat_request.with_system(&rendered);
                                system_state.reset_modified();
                            }
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
                // The Command tool now manages its own executing commands internally

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

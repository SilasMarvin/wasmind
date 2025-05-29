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
    tools::{InternalToolHandler, MCPExecutionStage, CommandExecutionStage, GeneralToolExecutionStage},
    tui::{self, events::FunctionExecution},
};


/// All available events the worker can handle
#[derive(Debug)]
pub enum Event {
    UserTUIInput(String),
    MCPToolsInit(Vec<Tool>),
    Action(Action),
    ChatResponse(MessageContent),
    MicrophoneResponse(String),
    // Specific stage updates for different tool types
    MCPStageUpdate {
        call_id: String,
        stage: MCPExecutionStage,
    },
    CommandStageUpdate {
        call_id: String,
        stage: CommandExecutionStage,
    },
    FileReaderStageUpdate {
        call_id: String,
        stage: GeneralToolExecutionStage,
    },
    FileEditorStageUpdate {
        call_id: String,
        stage: GeneralToolExecutionStage,
    },
    PlannerStageUpdate {
        call_id: String,
        stage: GeneralToolExecutionStage,
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
    let mut function_executions: std::collections::HashMap<String, FunctionExecution> =
        std::collections::HashMap::new();

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
                        // Create function executions for each tool call
                        for call in &tool_calls {
                            // Determine tool type and create appropriate execution
                            let tool_type = if tool_handler.is_internal_tool(&call.fn_name) {
                                // Determine specific internal tool type
                                match call.fn_name.as_str() {
                                    crate::tools::command::TOOL_NAME => {
                                        crate::tools::ToolType::Command(vec![CommandExecutionStage::Called {
                                            args: Some(call.fn_arguments.to_string()),
                                        }])
                                    }
                                    crate::tools::file_reader::TOOL_NAME => {
                                        crate::tools::ToolType::FileReader(vec![GeneralToolExecutionStage::Called {
                                            args: Some(call.fn_arguments.to_string()),
                                        }])
                                    }
                                    crate::tools::edit_file::TOOL_NAME => {
                                        crate::tools::ToolType::FileEditor(vec![GeneralToolExecutionStage::Called {
                                            args: Some(call.fn_arguments.to_string()),
                                        }])
                                    }
                                    crate::tools::planner::TOOL_NAME => {
                                        crate::tools::ToolType::Planner(vec![GeneralToolExecutionStage::Called {
                                            args: Some(call.fn_arguments.to_string()),
                                        }])
                                    }
                                    _ => {
                                        // Default to MCP if unknown internal tool
                                        crate::tools::ToolType::MCP(vec![MCPExecutionStage::Called {
                                            args: Some(call.fn_arguments.to_string()),
                                        }])
                                    }
                                }
                            } else {
                                // External MCP tool
                                crate::tools::ToolType::MCP(vec![MCPExecutionStage::Called {
                                    args: Some(call.fn_arguments.to_string()),
                                }])
                            };

                            let execution = FunctionExecution {
                                call_id: call.call_id.clone(),
                                name: call.fn_name.clone(),
                                tool_type,
                                start_time: chrono::Utc::now(),
                            };

                            // Store the execution
                            function_executions.insert(call.call_id.clone(), execution.clone());

                            // Send the initial update to TUI
                            let _ = tui_tx.send(tui::Task::AddEvent(
                                tui::events::TuiEvent::function_execution_update(execution),
                            ));
                        }

                        // Separate internal tools from MCP tools
                        let (internal_tools, mcp_tools): (Vec<_>, Vec<_>) =
                            tool_calls.into_iter().partition(|tool_call| {
                                tool_handler.is_internal_tool(&tool_call.fn_name)
                            });

                        // Handle internal tools
                        if !internal_tools.is_empty() {
                            // Internal tools will send their own stage updates
                            tool_handler.handle_tool_calls(internal_tools, &mut system_state);

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
            Event::MCPStageUpdate { call_id, stage } => {
                if let Some(execution) = function_executions.get_mut(&call_id) {
                    match &mut execution.tool_type {
                        crate::tools::ToolType::MCP(stages) => {
                            // Check if this is a completion stage
                            if let MCPExecutionStage::Completed { result } = &stage {
                                // Send tool response to assistant
                                let tool_responses = vec![ToolResponse {
                                    call_id: call_id.clone(),
                                    content: result.clone(),
                                }];
                                
                                chat_request = chat_request.append_message(ChatMessage {
                                    role: ChatRole::Tool,
                                    content: MessageContent::ToolResponses(tool_responses),
                                    options: None,
                                });
                                assistant_tx
                                    .send(assistant::Task::Assist(chat_request.clone()))
                                    .context(AssistantTaskSendSnafu)?;
                            } else if let MCPExecutionStage::Failed { error } = &stage {
                                // Send failure response to assistant
                                let tool_responses = vec![ToolResponse {
                                    call_id: call_id.clone(),
                                    content: error.clone(),
                                }];
                                
                                chat_request = chat_request.append_message(ChatMessage {
                                    role: ChatRole::Tool,
                                    content: MessageContent::ToolResponses(tool_responses),
                                    options: None,
                                });
                                assistant_tx
                                    .send(assistant::Task::Assist(chat_request.clone()))
                                    .context(AssistantTaskSendSnafu)?;
                            }
                            
                            stages.push(stage);
                        }
                        _ => {
                            error!("Received MCP stage update for non-MCP tool");
                        }
                    }
                    let _ = tui_tx.send(tui::Task::AddEvent(
                        tui::events::TuiEvent::function_execution_update(execution.clone()),
                    ));
                }
            }
            Event::CommandStageUpdate { call_id, stage } => {
                if let Some(execution) = function_executions.get_mut(&call_id) {
                    match &mut execution.tool_type {
                        crate::tools::ToolType::Command(stages) => {
                            // Check if this is a completion stage
                            if let CommandExecutionStage::Result { stdout, stderr, exit_code } = &stage {
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

                                // Send tool response to assistant
                                let tool_responses = vec![ToolResponse {
                                    call_id: call_id.clone(),
                                    content: result,
                                }];
                                
                                chat_request = chat_request.append_message(ChatMessage {
                                    role: ChatRole::Tool,
                                    content: MessageContent::ToolResponses(tool_responses),
                                    options: None,
                                });
                                assistant_tx
                                    .send(assistant::Task::Assist(chat_request.clone()))
                                    .context(AssistantTaskSendSnafu)?;
                            } else if let CommandExecutionStage::Failed { error } = &stage {
                                // Send failure response to assistant
                                let tool_responses = vec![ToolResponse {
                                    call_id: call_id.clone(),
                                    content: error.clone(),
                                }];
                                
                                chat_request = chat_request.append_message(ChatMessage {
                                    role: ChatRole::Tool,
                                    content: MessageContent::ToolResponses(tool_responses),
                                    options: None,
                                });
                                assistant_tx
                                    .send(assistant::Task::Assist(chat_request.clone()))
                                    .context(AssistantTaskSendSnafu)?;
                            }
                            
                            stages.push(stage);
                        }
                        _ => {
                            error!("Received Command stage update for non-Command tool");
                        }
                    }
                    let _ = tui_tx.send(tui::Task::AddEvent(
                        tui::events::TuiEvent::function_execution_update(execution.clone()),
                    ));
                }
            }
            Event::FileReaderStageUpdate { call_id, stage } => {
                if let Some(execution) = function_executions.get_mut(&call_id) {
                    match &mut execution.tool_type {
                        crate::tools::ToolType::FileReader(stages) => {
                            // Check if this is a completion stage
                            if let GeneralToolExecutionStage::Completed { result } = &stage {
                                // Send tool response to assistant
                                let tool_responses = vec![ToolResponse {
                                    call_id: call_id.clone(),
                                    content: result.clone(),
                                }];
                                
                                chat_request = chat_request.append_message(ChatMessage {
                                    role: ChatRole::Tool,
                                    content: MessageContent::ToolResponses(tool_responses),
                                    options: None,
                                });
                                assistant_tx
                                    .send(assistant::Task::Assist(chat_request.clone()))
                                    .context(AssistantTaskSendSnafu)?;
                            } else if let GeneralToolExecutionStage::Failed { error } = &stage {
                                // Send failure response to assistant
                                let tool_responses = vec![ToolResponse {
                                    call_id: call_id.clone(),
                                    content: error.clone(),
                                }];
                                
                                chat_request = chat_request.append_message(ChatMessage {
                                    role: ChatRole::Tool,
                                    content: MessageContent::ToolResponses(tool_responses),
                                    options: None,
                                });
                                assistant_tx
                                    .send(assistant::Task::Assist(chat_request.clone()))
                                    .context(AssistantTaskSendSnafu)?;
                            }
                            
                            stages.push(stage);
                        }
                        _ => {
                            error!("Received FileReader stage update for non-FileReader tool");
                        }
                    }
                    let _ = tui_tx.send(tui::Task::AddEvent(
                        tui::events::TuiEvent::function_execution_update(execution.clone()),
                    ));
                }
            }
            Event::FileEditorStageUpdate { call_id, stage } => {
                if let Some(execution) = function_executions.get_mut(&call_id) {
                    match &mut execution.tool_type {
                        crate::tools::ToolType::FileEditor(stages) => {
                            // Check if this is a completion stage
                            if let GeneralToolExecutionStage::Completed { result } = &stage {
                                // Send tool response to assistant
                                let tool_responses = vec![ToolResponse {
                                    call_id: call_id.clone(),
                                    content: result.clone(),
                                }];
                                
                                chat_request = chat_request.append_message(ChatMessage {
                                    role: ChatRole::Tool,
                                    content: MessageContent::ToolResponses(tool_responses),
                                    options: None,
                                });
                                assistant_tx
                                    .send(assistant::Task::Assist(chat_request.clone()))
                                    .context(AssistantTaskSendSnafu)?;
                            } else if let GeneralToolExecutionStage::Failed { error } = &stage {
                                // Send failure response to assistant
                                let tool_responses = vec![ToolResponse {
                                    call_id: call_id.clone(),
                                    content: error.clone(),
                                }];
                                
                                chat_request = chat_request.append_message(ChatMessage {
                                    role: ChatRole::Tool,
                                    content: MessageContent::ToolResponses(tool_responses),
                                    options: None,
                                });
                                assistant_tx
                                    .send(assistant::Task::Assist(chat_request.clone()))
                                    .context(AssistantTaskSendSnafu)?;
                            }
                            
                            stages.push(stage);
                        }
                        _ => {
                            error!("Received FileEditor stage update for non-FileEditor tool");
                        }
                    }
                    let _ = tui_tx.send(tui::Task::AddEvent(
                        tui::events::TuiEvent::function_execution_update(execution.clone()),
                    ));
                }
            }
            Event::PlannerStageUpdate { call_id, stage } => {
                if let Some(execution) = function_executions.get_mut(&call_id) {
                    match &mut execution.tool_type {
                        crate::tools::ToolType::Planner(stages) => {
                            // Check if this is a completion stage
                            if let GeneralToolExecutionStage::Completed { result } = &stage {
                                // Send tool response to assistant
                                let tool_responses = vec![ToolResponse {
                                    call_id: call_id.clone(),
                                    content: result.clone(),
                                }];
                                
                                chat_request = chat_request.append_message(ChatMessage {
                                    role: ChatRole::Tool,
                                    content: MessageContent::ToolResponses(tool_responses),
                                    options: None,
                                });
                                assistant_tx
                                    .send(assistant::Task::Assist(chat_request.clone()))
                                    .context(AssistantTaskSendSnafu)?;
                            } else if let GeneralToolExecutionStage::Failed { error } = &stage {
                                // Send failure response to assistant
                                let tool_responses = vec![ToolResponse {
                                    call_id: call_id.clone(),
                                    content: error.clone(),
                                }];
                                
                                chat_request = chat_request.append_message(ChatMessage {
                                    role: ChatRole::Tool,
                                    content: MessageContent::ToolResponses(tool_responses),
                                    options: None,
                                });
                                assistant_tx
                                    .send(assistant::Task::Assist(chat_request.clone()))
                                    .context(AssistantTaskSendSnafu)?;
                            }
                            
                            stages.push(stage);
                        }
                        _ => {
                            error!("Received Planner stage update for non-Planner tool");
                        }
                    }
                    let _ = tui_tx.send(tui::Task::AddEvent(
                        tui::events::TuiEvent::function_execution_update(execution.clone()),
                    ));
                }
            }
        }
    }

    Ok(())
}

pub mod command;
pub mod edit_file;
pub mod file_reader;
pub mod planner;

use crossbeam::channel::Sender;
use genai::chat::{ToolCall, ToolResponse};
use serde_json::Value;
use tracing::error;

use crate::{config::ParsedConfig, tui, worker::Event};

/// Handler for all internal tools
pub struct InternalToolHandler {
    command: command::Command,
    planner: planner::Planner,
    file_reader: file_reader::FileReader,
    file_editor: edit_file::FileEditor,
    worker_tx: Sender<Event>,
    tui_tx: Sender<tui::Task>,
    config: ParsedConfig,
}

impl InternalToolHandler {
    /// Create a new internal tool handler
    pub fn new(worker_tx: Sender<Event>, tui_tx: Sender<tui::Task>, config: ParsedConfig) -> Self {
        Self {
            command: command::Command::new(worker_tx.clone(), config.clone()),
            planner: planner::Planner::new(),
            file_reader: file_reader::FileReader::new(),
            file_editor: edit_file::FileEditor::new(),
            worker_tx,
            tui_tx,
            config,
        }
    }

    /// Check if a tool name is an internal tool
    pub fn is_internal_tool(&self, tool_name: &str) -> bool {
        matches!(tool_name, command::TOOL_NAME | planner::TOOL_NAME | file_reader::TOOL_NAME | edit_file::TOOL_NAME)
    }

    /// Get the list of all internal tool names
    pub fn get_tool_names(&self) -> Vec<String> {
        vec![
            command::TOOL_NAME.to_string(),
            planner::TOOL_NAME.to_string(),
            file_reader::TOOL_NAME.to_string(),
            edit_file::TOOL_NAME.to_string(),
        ]
    }
    
    /// Get tool info by name (name, description, schema)
    pub fn get_tool_info(&self, name: &str) -> Option<(&'static str, &'static str, Value)> {
        match name {
            command::TOOL_NAME => Some((
                command::TOOL_NAME,
                command::TOOL_DESCRIPTION,
                serde_json::from_str(command::TOOL_INPUT_SCHEMA).unwrap()
            )),
            planner::TOOL_NAME => Some((
                planner::TOOL_NAME,
                planner::TOOL_DESCRIPTION,
                serde_json::from_str(planner::TOOL_INPUT_SCHEMA).unwrap()
            )),
            file_reader::TOOL_NAME => Some((
                file_reader::TOOL_NAME,
                file_reader::TOOL_DESCRIPTION,
                serde_json::from_str(file_reader::TOOL_INPUT_SCHEMA).unwrap()
            )),
            edit_file::TOOL_NAME => Some((
                edit_file::TOOL_NAME,
                edit_file::TOOL_DESCRIPTION,
                serde_json::from_str(edit_file::TOOL_INPUT_SCHEMA).unwrap()
            )),
            _ => None,
        }
    }

    /// Handle a batch of tool calls
    pub fn handle_tool_calls(&mut self, tool_calls: Vec<ToolCall>) -> Vec<ToolResponse> {
        let mut responses = Vec::new();

        for tool_call in tool_calls {
            let response = match tool_call.fn_name.as_str() {
                command::TOOL_NAME => {
                    match self.command.handle_call(tool_call.clone(), &self.tui_tx) {
                        Ok(Some(response)) => response,
                        Ok(None) => continue, // Tool handled the call but doesn't need to send a response
                        Err(e) => {
                            error!("Error handling command tool call: {}", e);
                            ToolResponse {
                                call_id: tool_call.call_id,
                                content: format!("Error: {}", e),
                            }
                        }
                    }
                }
                planner::TOOL_NAME => {
                    match self.planner.handle_call(tool_call.clone(), &self.tui_tx) {
                        Ok(Some(response)) => response,
                        Ok(None) => continue,
                        Err(e) => {
                            error!("Error handling planner tool call: {}", e);
                            ToolResponse {
                                call_id: tool_call.call_id,
                                content: format!("Error: {}", e),
                            }
                        }
                    }
                }
                file_reader::TOOL_NAME => {
                    // Handle file reader tool call
                    let args = &tool_call.fn_arguments;
                    
                    let path = match args.get("path").and_then(|p| p.as_str()) {
                        Some(p) => p,
                        None => {
                            responses.push(ToolResponse {
                                call_id: tool_call.call_id,
                                content: "Error: 'path' parameter is required".to_string(),
                            });
                            continue;
                        }
                    };
                    
                    match self.file_reader.get_or_read_file_content(path) {
                        Ok(content) => ToolResponse {
                            call_id: tool_call.call_id,
                            content: content.clone(),
                        },
                        Err(e) => ToolResponse {
                            call_id: tool_call.call_id,
                            content: format!("Error reading file: {}", e),
                        }
                    }
                }
                edit_file::TOOL_NAME => {
                    // Handle file editor tool call
                    let args = &tool_call.fn_arguments;
                    
                    let path = match args.get("path").and_then(|p| p.as_str()) {
                        Some(p) => p,
                        None => {
                            responses.push(ToolResponse {
                                call_id: tool_call.call_id,
                                content: "Error: 'path' parameter is required".to_string(),
                            });
                            continue;
                        }
                    };
                    
                    match edit_file::FileEditor::parse_action_from_args(args) {
                        Ok(action) => {
                            match self.file_editor.edit_file(path, action, &mut self.file_reader) {
                                Ok(message) => ToolResponse {
                                    call_id: tool_call.call_id,
                                    content: message,
                                },
                                Err(e) => ToolResponse {
                                    call_id: tool_call.call_id,
                                    content: format!("Error editing file: {}", e),
                                }
                            }
                        }
                        Err(e) => ToolResponse {
                            call_id: tool_call.call_id,
                            content: format!("Error parsing edit action: {}", e),
                        }
                    }
                }
                _ => {
                    error!("Unknown internal tool: {}", tool_call.fn_name);
                    ToolResponse {
                        call_id: tool_call.call_id,
                        content: format!("Unknown internal tool: {}", tool_call.fn_name),
                    }
                }
            };

            responses.push(response);
        }

        responses
    }

    /// Get the current task plan from the planner (if any)
    pub fn get_current_task_plan(&self) -> Option<planner::TaskPlan> {
        self.planner.get_current_plan().cloned()
    }

    /// Handle user input that might affect tools (e.g., command confirmation)
    /// Returns true if the input was consumed by a tool
    pub fn handle_user_input(&mut self, input: &str) -> Result<bool, String> {
        // The command tool will handle its own pending commands
        if self.command.has_pending_command() {
            if let Some((command, args, tool_call_id, _)) = self.command.handle_user_confirmation(input) {
                // User denied - send denial response
                let _ = self.worker_tx.send(Event::CommandExecutionResult {
                    tool_call_id,
                    command: format!("{} {}", command, args.join(" ")),
                    stdout: String::new(),
                    stderr: "Command execution denied by user".to_string(),
                    exit_code: -1,
                });
            }

            // Send UI updates
            let _ = self.tui_tx.send(tui::Task::ClearInput);
            let _ = self.tui_tx.send(tui::Task::AddEvent(
                tui::events::TuiEvent::set_waiting_for_confirmation(false),
            ));

            return Ok(true); // Input was consumed
        }
        Ok(false) // Input was not consumed
    }

    /// Cancel any pending operations
    pub fn cancel_pending_operations(&mut self) {
        self.command.cancel_pending_operations();
    }
}


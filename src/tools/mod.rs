pub mod command;
pub mod edit_file;
pub mod file_reader;
pub mod planner;

use crossbeam::channel::Sender;
use genai::chat::ToolCall;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::error;

use crate::{config::ParsedConfig, system_state::SystemState, tui, worker::Event};

/// General execution stages for simple tools that don't need detailed tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GeneralToolExecutionStage {
    Called { args: Option<String> },
    Completed { result: String },
    Failed { error: String },
}

/// Execution stages specific to MCP (Model Context Protocol) tools
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MCPExecutionStage {
    Called { args: Option<String> },
    Completed { result: String },
    Failed { error: String },
}

/// Execution stages specific to the Command tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandExecutionStage {
    Called {
        args: Option<String>,
    },
    AwaitingApproval {
        command: String,
        args: Vec<String>,
    },
    Executing {
        command: String,
    },
    Result {
        stdout: String,
        stderr: String,
        exit_code: i32,
    },
    Failed {
        error: String,
    },
}

/// Represents different tool types with their specific execution stages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolType {
    MCP(Vec<MCPExecutionStage>),
    Command(Vec<CommandExecutionStage>),
    FileReader(Vec<GeneralToolExecutionStage>),
    FileEditor(Vec<GeneralToolExecutionStage>),
    Planner(Vec<GeneralToolExecutionStage>),
}

impl ToolType {
    /// Returns true if the execution is complete (either successfully or failed)
    pub fn is_complete(&self) -> bool {
        match self {
            ToolType::MCP(stages) => stages.iter().any(|stage| {
                matches!(
                    stage,
                    MCPExecutionStage::Completed { .. } | MCPExecutionStage::Failed { .. }
                )
            }),
            ToolType::Command(stages) => stages.iter().any(|stage| {
                matches!(
                    stage,
                    CommandExecutionStage::Result { .. } | CommandExecutionStage::Failed { .. }
                )
            }),
            ToolType::FileReader(stages)
            | ToolType::FileEditor(stages)
            | ToolType::Planner(stages) => stages.iter().any(|stage| {
                matches!(
                    stage,
                    GeneralToolExecutionStage::Completed { .. }
                        | GeneralToolExecutionStage::Failed { .. }
                )
            }),
        }
    }

    /// Returns a user-friendly description of the current stage
    pub fn current_stage_description(&self) -> Option<String> {
        match self {
            ToolType::MCP(stages) => stages.last().map(|stage| match stage {
                MCPExecutionStage::Called { .. } => "Called".to_string(),
                MCPExecutionStage::Completed { .. } => "Completed".to_string(),
                MCPExecutionStage::Failed { error } => format!("Failed: {}", error),
            }),
            ToolType::Command(stages) => stages.last().map(|stage| match stage {
                CommandExecutionStage::Called { .. } => "Called".to_string(),
                CommandExecutionStage::AwaitingApproval { command, .. } => {
                    format!("Awaiting approval for: {}", command)
                }
                CommandExecutionStage::Executing { command } => format!("Executing: {}", command),
                CommandExecutionStage::Result { exit_code, .. } => {
                    format!("Completed with exit code: {}", exit_code)
                }
                CommandExecutionStage::Failed { error } => format!("Failed: {}", error),
            }),
            ToolType::FileReader(stages)
            | ToolType::FileEditor(stages)
            | ToolType::Planner(stages) => stages.last().map(|stage| match stage {
                GeneralToolExecutionStage::Called { .. } => "Called".to_string(),
                GeneralToolExecutionStage::Completed { .. } => "Completed".to_string(),
                GeneralToolExecutionStage::Failed { error } => format!("Failed: {}", error),
            }),
        }
    }
}

/// Handler for all internal tools
pub struct InternalToolHandler {
    command: command::Command,
    planner: planner::Planner,
    file_reader: file_reader::FileReader,
    file_editor: edit_file::FileEditor,
    worker_tx: Sender<Event>,
    tui_tx: Sender<tui::Task>,
    _config: ParsedConfig,
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
            _config: config,
        }
    }

    /// Check if a tool name is an internal tool
    pub fn is_internal_tool(&self, tool_name: &str) -> bool {
        matches!(
            tool_name,
            command::TOOL_NAME | planner::TOOL_NAME | file_reader::TOOL_NAME | edit_file::TOOL_NAME
        )
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
                serde_json::from_str(command::TOOL_INPUT_SCHEMA).unwrap(),
            )),
            planner::TOOL_NAME => Some((
                planner::TOOL_NAME,
                planner::TOOL_DESCRIPTION,
                serde_json::from_str(planner::TOOL_INPUT_SCHEMA).unwrap(),
            )),
            file_reader::TOOL_NAME => Some((
                file_reader::TOOL_NAME,
                file_reader::TOOL_DESCRIPTION,
                serde_json::from_str(file_reader::TOOL_INPUT_SCHEMA).unwrap(),
            )),
            edit_file::TOOL_NAME => Some((
                edit_file::TOOL_NAME,
                edit_file::TOOL_DESCRIPTION,
                serde_json::from_str(edit_file::TOOL_INPUT_SCHEMA).unwrap(),
            )),
            _ => None,
        }
    }

    /// Handle a batch of tool calls and update system state
    pub fn handle_tool_calls(
        &mut self,
        tool_calls: Vec<ToolCall>,
        system_state: &mut SystemState,
    ) {
        for tool_call in tool_calls {
            match tool_call.fn_name.as_str() {
                command::TOOL_NAME => {
                    match self.command.handle_call(tool_call.clone(), &self.tui_tx) {
                        Ok(_) => {}, // Command tool sends its own stage updates
                        Err(e) => {
                            error!("Error handling command tool call: {}", e);
                            // Send failure stage update
                            let _ = self.worker_tx.send(Event::CommandStageUpdate {
                                call_id: tool_call.call_id,
                                stage: CommandExecutionStage::Failed {
                                    error: format!("Error: {}", e),
                                },
                            });
                        }
                    }
                }
                planner::TOOL_NAME => {
                    match self.planner.handle_call(tool_call.clone(), &self.tui_tx) {
                        Ok(Some(_response)) => {
                            // Update system state with the current plan (no conversion needed)
                            if let Some(plan) = self.planner.get_current_plan() {
                                system_state.update_plan(plan.clone());
                            }

                            // Send success stage update
                            let _ = self.worker_tx.send(Event::PlannerStageUpdate {
                                call_id: tool_call.call_id,
                                stage: GeneralToolExecutionStage::Completed {
                                    result: "Plan updated successfully. Check the system context for current plan details.".to_string(),
                                },
                            });
                        }
                        Ok(None) => {},
                        Err(e) => {
                            error!("Error handling planner tool call: {}", e);
                            // Send failure stage update
                            let _ = self.worker_tx.send(Event::PlannerStageUpdate {
                                call_id: tool_call.call_id,
                                stage: GeneralToolExecutionStage::Failed {
                                    error: format!("Error: {}", e),
                                },
                            });
                        }
                    }
                }
                file_reader::TOOL_NAME => {
                    // Handle file reader tool call
                    let args = &tool_call.fn_arguments;

                    let path = match args.get("path").and_then(|p| p.as_str()) {
                        Some(p) => p,
                        None => {
                            let _ = self.worker_tx.send(Event::FileReaderStageUpdate {
                                call_id: tool_call.call_id,
                                stage: GeneralToolExecutionStage::Failed {
                                    error: "Error: 'path' parameter is required".to_string(),
                                },
                            });
                            continue;
                        }
                    };

                    match self.file_reader.get_or_read_file_content(path) {
                        Ok(content) => {
                            // Update system state with the file content
                            let path_buf = std::path::PathBuf::from(path);
                            if let Ok(metadata) = std::fs::metadata(&path_buf) {
                                if let Ok(modified) = metadata.modified() {
                                    system_state.update_file(
                                        path_buf.clone(),
                                        content.clone(),
                                        modified,
                                    );
                                }
                            }

                            let _ = self.worker_tx.send(Event::FileReaderStageUpdate {
                                call_id: tool_call.call_id,
                                stage: GeneralToolExecutionStage::Completed {
                                    result: format!(
                                        "Successfully read file: {} ({} lines)",
                                        path,
                                        content.lines().count()
                                    ),
                                },
                            });
                        }
                        Err(e) => {
                            let _ = self.worker_tx.send(Event::FileReaderStageUpdate {
                                call_id: tool_call.call_id,
                                stage: GeneralToolExecutionStage::Failed {
                                    error: format!("Error reading file: {}", e),
                                },
                            });
                        }
                    }
                }
                edit_file::TOOL_NAME => {
                    // Handle file editor tool call
                    let args = &tool_call.fn_arguments;

                    let path = match args.get("path").and_then(|p| p.as_str()) {
                        Some(p) => p,
                        None => {
                            let _ = self.worker_tx.send(Event::FileEditorStageUpdate {
                                call_id: tool_call.call_id,
                                stage: GeneralToolExecutionStage::Failed {
                                    error: "Error: 'path' parameter is required".to_string(),
                                },
                            });
                            continue;
                        }
                    };

                    match edit_file::FileEditor::parse_action_from_args(args) {
                        Ok(action) => {
                            match self
                                .file_editor
                                .edit_file(path, action, &mut self.file_reader)
                            {
                                Ok(message) => {
                                    // Update system state with the new file content
                                    let path_buf = std::path::PathBuf::from(path);
                                    if let Ok(new_content) =
                                        self.file_reader.get_or_read_file_content(path)
                                    {
                                        if let Ok(metadata) = std::fs::metadata(&path_buf) {
                                            if let Ok(modified) = metadata.modified() {
                                                system_state.update_file(
                                                    path_buf,
                                                    new_content.clone(),
                                                    modified,
                                                );
                                            }
                                        }
                                    }

                                    let _ = self.worker_tx.send(Event::FileEditorStageUpdate {
                                        call_id: tool_call.call_id,
                                        stage: GeneralToolExecutionStage::Completed {
                                            result: message,
                                        },
                                    });
                                }
                                Err(e) => {
                                    let _ = self.worker_tx.send(Event::FileEditorStageUpdate {
                                        call_id: tool_call.call_id,
                                        stage: GeneralToolExecutionStage::Failed {
                                            error: format!("Error editing file: {}", e),
                                        },
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            let _ = self.worker_tx.send(Event::FileEditorStageUpdate {
                                call_id: tool_call.call_id,
                                stage: GeneralToolExecutionStage::Failed {
                                    error: format!("Error parsing edit action: {}", e),
                                },
                            });
                        }
                    }
                }
                _ => {
                    error!("Unknown internal tool: {}", tool_call.fn_name);
                    // This shouldn't happen but send a failure stage update anyway
                    let _ = self.worker_tx.send(Event::FileReaderStageUpdate {
                        call_id: tool_call.call_id,
                        stage: GeneralToolExecutionStage::Failed {
                            error: format!("Unknown internal tool: {}", tool_call.fn_name),
                        },
                    });
                }
            }
        }
    }

    /// Get the current task plan from the planner (if any)
    pub fn get_current_task_plan(&self) -> Option<crate::tools::planner::TaskPlan> {
        self.planner.get_current_plan().cloned()
    }

    /// Handle user input that might affect tools (e.g., command confirmation)
    /// Returns true if the input was consumed by a tool
    pub fn handle_user_input(&mut self, input: &str) -> Result<bool, String> {
        // The command tool will handle its own pending commands
        if self.command.has_pending_command() {
            if let Some((_command, _args, tool_call_id, _)) =
                self.command.handle_user_confirmation(input)
            {
                // User denied - send failure stage update
                let _ = self.worker_tx.send(Event::CommandStageUpdate {
                    call_id: tool_call_id,
                    stage: CommandExecutionStage::Failed {
                        error: "Command execution denied by user".to_string(),
                    },
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

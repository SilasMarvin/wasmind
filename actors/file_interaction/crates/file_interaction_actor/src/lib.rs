use bindings::{
    exports::wasmind::actor::actor::MessageEnvelope, 
    wasmind::actor::host_info
};
use file_interaction::{
    EDIT_FILE_DESCRIPTION, EDIT_FILE_NAME, EDIT_FILE_SCHEMA, EditFileParams,
    FILE_TOOLS_USAGE_GUIDE, FileInteractionManager, READ_FILE_DESCRIPTION, READ_FILE_NAME,
    READ_FILE_SCHEMA, ReadFileParams,
};
use std::path::PathBuf;
use serde::Deserialize;
use wasmind_actor_utils::common_messages::{
    assistant::{Section, SystemPromptContent, SystemPromptContribution},
    tools::{
        ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, ToolsAvailable,
        UIDisplayInfo,
    },
};

#[allow(warnings)]
mod bindings;

#[derive(Deserialize)]
struct FileInteractionActorConfig {
    allow_edits: bool,
}

impl Default for FileInteractionActorConfig {
    fn default() -> Self {
        FileInteractionActorConfig { allow_edits: true }
    }
}

wasmind_actor_utils::actors::macros::generate_actor_trait!();

#[derive(wasmind_actor_utils::actors::macros::Actor)]
pub struct FileInteractionActor {
    scope: String,
    manager: FileInteractionManager,
    config: FileInteractionActorConfig,
}

impl GeneratedActorTrait for FileInteractionActor {
    fn new(scope: String, config_str: String) -> Self {
        let config: FileInteractionActorConfig = toml::from_str(&config_str).unwrap_or_default();

        let tools = if config.allow_edits {
            vec![
                wasmind_actor_utils::llm_client_types::Tool {
                    tool_type: "function".to_string(),
                    function: wasmind_actor_utils::llm_client_types::ToolFunctionDefinition {
                        name: READ_FILE_NAME.to_string(),
                        description: READ_FILE_DESCRIPTION.to_string(),
                        parameters: serde_json::from_str(READ_FILE_SCHEMA).unwrap(),
                    },
                },
                wasmind_actor_utils::llm_client_types::Tool {
                    tool_type: "function".to_string(),
                    function: wasmind_actor_utils::llm_client_types::ToolFunctionDefinition {
                        name: EDIT_FILE_NAME.to_string(),
                        description: EDIT_FILE_DESCRIPTION.to_string(),
                        parameters: serde_json::from_str(EDIT_FILE_SCHEMA).unwrap(),
                    },
                },
            ]
        } else {
            vec![wasmind_actor_utils::llm_client_types::Tool {
                tool_type: "function".to_string(),
                function: wasmind_actor_utils::llm_client_types::ToolFunctionDefinition {
                    name: READ_FILE_NAME.to_string(),
                    description: READ_FILE_DESCRIPTION.to_string(),
                    parameters: serde_json::from_str(READ_FILE_SCHEMA).unwrap(),
                },
            }]
        };

        let _ = Self::broadcast_common_message(ToolsAvailable { tools });

        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "file_interaction:usage_guide".to_string(),
            content: SystemPromptContent::Text(FILE_TOOLS_USAGE_GUIDE.to_string()),
            priority: 900,
            section: Some(Section::Tools),
        });

        // Get the host working directory and create the manager with it
        let working_directory = host_info::get_host_working_directory();
        let manager = FileInteractionManager::new_with_working_directory(PathBuf::from(working_directory));

        Self {
            scope,
            manager,
            config,
        }
    }

    fn handle_message(&mut self, message: MessageEnvelope) {
        if message.from_scope != self.scope {
            return;
        }

        if let Some(execute_tool) = Self::parse_as::<ExecuteTool>(&message) {
            match execute_tool.tool_call.function.name.as_str() {
                READ_FILE_NAME => self.handle_read_file(execute_tool),
                EDIT_FILE_NAME if self.config.allow_edits => self.handle_edit_file(execute_tool),
                _ => {}
            }
        }
    }

    fn destructor(&mut self) {
        // Clear cache on destruction
        self.manager.clear_cache();
    }
}

impl FileInteractionActor {
    fn update_unified_files_system_prompt(&self) {
        let files_info = self.manager.get_files_info();
        let mut files = Vec::new();

        for (path, content) in files_info {
            files.push(serde_json::json!({
                "path": path.display().to_string(),
                "content": content
            }));
        }

        files.sort_by(|a, b| {
            a["path"]
                .as_str()
                .unwrap_or("")
                .cmp(b["path"].as_str().unwrap_or(""))
        });

        // Get the current working directory
        let working_directory = host_info::get_host_working_directory();
        
        let data = serde_json::json!({
            "working_directory": working_directory,
            "files": files
        });

        let default_template = r#"Current Working Directory: {{ data.working_directory }}

The current state of all read and edited files. This is updated automatically for you after each edit_file and read_file call. I.E. You do NOT need to call read_file after edit_file uses

{% for file in data.files -%}
<file path="{{ file.path }}">
{{ file.content }}
</file>
{% endfor %}"#
            .to_string();

        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: self.scope.clone(),
            key: "file_interaction:files_read_and_edited".to_string(),
            content: SystemPromptContent::Data {
                data,
                default_template,
            },
            priority: 500,
            section: Some(Section::Custom("FilesReadAndEdited".to_string())),
        });
    }

    fn handle_read_file(&mut self, execute_tool: ExecuteTool) {
        let tool_call_id = &execute_tool.tool_call.id;
        let originating_request_id = &execute_tool.originating_request_id;

        let params: ReadFileParams =
            match serde_json::from_str(&execute_tool.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    self.send_error_result(
                        tool_call_id,
                        originating_request_id,
                        format!("Failed to parse read_file parameters: {e}"),
                        UIDisplayInfo {
                            collapsed: "Parameters: Invalid format".to_string(),
                            expanded: Some(format!(
                                "Error: Failed to parse parameters\n\nDetails: {e}"
                            )),
                        },
                    );
                    return;
                }
            };

        match self.manager.read_file(params) {
            Ok(result) => {
                self.update_unified_files_system_prompt();
                let message = format!(
                    "{} -- Check the FilesReadAndEdited section in the SystemPrompt to see the read file",
                    result.message
                );
                self.send_success_result(
                    tool_call_id,
                    originating_request_id,
                    message,
                    result.ui_display,
                );
            }
            Err(error) => {
                self.send_error_result(
                    tool_call_id,
                    originating_request_id,
                    error.error_msg,
                    error.ui_display,
                );
            }
        }
    }

    fn handle_edit_file(&mut self, execute_tool: ExecuteTool) {
        let tool_call_id = &execute_tool.tool_call.id;
        let originating_request_id = &execute_tool.originating_request_id;

        let params: EditFileParams =
            match serde_json::from_str(&execute_tool.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    self.send_error_result(
                        tool_call_id,
                        originating_request_id,
                        format!("Failed to parse edit_file parameters: {e}"),
                        UIDisplayInfo {
                            collapsed: "Parameters: Invalid format".to_string(),
                            expanded: Some(format!(
                                "Error: Failed to parse parameters\n\nDetails: {e}"
                            )),
                        },
                    );
                    return;
                }
            };

        match self.manager.edit_file(&params) {
            Ok(result) => {
                self.update_unified_files_system_prompt();
                let message = format!(
                    "{} -- Check the FilesReadAndEdited section in the SystemPrompt to see the updated edited file",
                    result.message
                );
                self.send_success_result(
                    tool_call_id,
                    originating_request_id,
                    message,
                    result.ui_display,
                );
            }
            Err(error) => {
                self.send_error_result(
                    tool_call_id,
                    originating_request_id,
                    error.error_msg,
                    error.ui_display,
                );
            }
        }
    }

    fn send_error_result(
        &self,
        tool_call_id: &str,
        originating_request_id: &str,
        error_msg: String,
        ui_display: UIDisplayInfo,
    ) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: originating_request_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(ToolCallResult {
                    content: error_msg,
                    ui_display_info: ui_display,
                }),
            },
        };

        let _ = Self::broadcast_common_message(update);
    }

    fn send_success_result(
        &self,
        tool_call_id: &str,
        originating_request_id: &str,
        result: String,
        ui_display: UIDisplayInfo,
    ) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: originating_request_id.to_string(),
            status: ToolCallStatus::Done {
                result: Ok(ToolCallResult {
                    content: result,
                    ui_display_info: ui_display,
                }),
            },
        };

        let _ = Self::broadcast_common_message(update);
    }
}

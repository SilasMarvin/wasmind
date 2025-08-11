use bindings::exports::hive::actor::actor::MessageEnvelope;
use file_interaction::{
    EDIT_FILE_DESCRIPTION, EDIT_FILE_NAME, EDIT_FILE_SCHEMA, EditFileParams,
    FILE_TOOLS_USAGE_GUIDE, FileInteractionManager, READ_FILE_DESCRIPTION, READ_FILE_NAME,
    READ_FILE_SCHEMA, ReadFileParams,
};
use hive_actor_utils::common_messages::{
    assistant::{Section, SystemPromptContent, SystemPromptContribution},
    tools::{
        ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, ToolsAvailable,
        UIDisplayInfo,
    },
};
use serde::Deserialize;

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

hive_actor_utils::actors::macros::generate_actor_trait!();

#[derive(hive_actor_utils::actors::macros::Actor)]
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
                hive_actor_utils::llm_client_types::Tool {
                    tool_type: "function".to_string(),
                    function: hive_actor_utils::llm_client_types::ToolFunctionDefinition {
                        name: READ_FILE_NAME.to_string(),
                        description: READ_FILE_DESCRIPTION.to_string(),
                        parameters: serde_json::from_str(READ_FILE_SCHEMA).unwrap(),
                    },
                },
                hive_actor_utils::llm_client_types::Tool {
                    tool_type: "function".to_string(),
                    function: hive_actor_utils::llm_client_types::ToolFunctionDefinition {
                        name: EDIT_FILE_NAME.to_string(),
                        description: EDIT_FILE_DESCRIPTION.to_string(),
                        parameters: serde_json::from_str(EDIT_FILE_SCHEMA).unwrap(),
                    },
                },
            ]
        } else {
            vec![hive_actor_utils::llm_client_types::Tool {
                tool_type: "function".to_string(),
                function: hive_actor_utils::llm_client_types::ToolFunctionDefinition {
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

        Self {
            scope,
            manager: FileInteractionManager::new(),
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

        let data = serde_json::json!({
            "files": files
        });

        let default_template = r#"{% for file in data.files -%}
<file path="{{ file.path }}">{{ file.content }}</file>
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

        let params: ReadFileParams =
            match serde_json::from_str(&execute_tool.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    self.send_error_result(
                        tool_call_id,
                        format!("Failed to parse read_file parameters: {}", e),
                        UIDisplayInfo {
                            collapsed: "Parameters: Invalid format".to_string(),
                            expanded: Some(format!(
                                "Error: Failed to parse parameters\n\nDetails: {}",
                                e
                            )),
                        },
                    );
                    return;
                }
            };

        match self.manager.read_file(params) {
            Ok(result) => {
                self.update_unified_files_system_prompt();
                self.send_success_result(tool_call_id, result.message, result.ui_display);
            }
            Err(error) => {
                self.send_error_result(tool_call_id, error.error_msg, error.ui_display);
            }
        }
    }

    fn handle_edit_file(&mut self, execute_tool: ExecuteTool) {
        let tool_call_id = &execute_tool.tool_call.id;

        let params: EditFileParams =
            match serde_json::from_str(&execute_tool.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    self.send_error_result(
                        tool_call_id,
                        format!("Failed to parse edit_file parameters: {}", e),
                        UIDisplayInfo {
                            collapsed: "Parameters: Invalid format".to_string(),
                            expanded: Some(format!(
                                "Error: Failed to parse parameters\n\nDetails: {}",
                                e
                            )),
                        },
                    );
                    return;
                }
            };

        match self.manager.edit_file(&params) {
            Ok(result) => {
                self.update_unified_files_system_prompt();
                self.send_success_result(tool_call_id, result.message, result.ui_display);
            }
            Err(error) => {
                self.send_error_result(tool_call_id, error.error_msg, error.ui_display);
            }
        }
    }

    fn send_error_result(&self, tool_call_id: &str, error_msg: String, ui_display: UIDisplayInfo) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(ToolCallResult {
                    content: error_msg,
                    ui_display_info: ui_display,
                }),
            },
        };

        let _ = Self::broadcast_common_message(update);
    }

    fn send_success_result(&self, tool_call_id: &str, result: String, ui_display: UIDisplayInfo) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
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

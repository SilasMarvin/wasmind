use std::collections::HashMap;

use bindings::{
    exports::hive::actor::actor::MessageEnvelope,
    hive::actor::{actor::Scope, agent::spawn_agent, logger},
};
use code_with_experts_common::ApprovalResponse;
use file_interaction::{
    EDIT_FILE_DESCRIPTION, EDIT_FILE_NAME, EDIT_FILE_SCHEMA, EditFileParams,
    FILE_TOOLS_USAGE_GUIDE, FileInteractionManager, READ_FILE_DESCRIPTION, READ_FILE_NAME,
    READ_FILE_SCHEMA, ReadFileParams,
};
use hive_actor_utils::{
    common_messages::{
        assistant::{AddMessage, Section, SystemPromptContent, SystemPromptContribution},
        tools::{
            ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, ToolsAvailable,
            UIDisplayInfo,
        },
    },
    llm_client_types::{ChatMessage, SystemChatMessage},
};
use serde::Deserialize;

#[allow(warnings)]
mod bindings;

#[derive(Deserialize)]
struct ApprovalConfig {
    approvers: HashMap<String, Vec<String>>,
}

hive_actor_utils::actors::macros::generate_actor_trait!();

struct ActiveEditFileCall {
    tool_call_id: String,
    edit_file_params: EditFileParams,
    approver_responses: HashMap<Scope, Option<ApprovalResponse>>,
}

#[derive(hive_actor_utils::actors::macros::Actor)]
pub struct FileInteractionWIthApprovalActor {
    scope: String,
    manager: FileInteractionManager,
    approver_scopes: Vec<Scope>,
    active_edit_file_call: Option<ActiveEditFileCall>,
}

impl GeneratedActorTrait for FileInteractionWIthApprovalActor {
    fn new(scope: String, config_str: String) -> Self {
        let config: ApprovalConfig =
            toml::from_str(&config_str).expect("Error deserializing config");

        let approver_scopes = config
            .approvers
            .into_iter()
            .map(|(approver_name, mut approver_actors)| {
                approver_actors.extend_from_slice(&[
                    "hcwe_approve".to_string(),
                    "hcwe_request_changes".to_string(),
                ]);
                spawn_agent(&approver_actors, &approver_name)
                    .expect("Error spawning initial actors")
            })
            .collect();

        let tools = vec![
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
        ];
        let _ = Self::broadcast_common_message(ToolsAvailable { tools });

        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "file_interaction:usage_guide".to_string(),
            content: SystemPromptContent::Text(FILE_TOOLS_USAGE_GUIDE.to_string()),
            priority: 900,
            section: Some(Section::Tools),
        });

        Self {
            scope: scope.clone(),
            manager: FileInteractionManager::new(),
            approver_scopes,
            active_edit_file_call: None,
        }
    }

    fn handle_message(&mut self, message: MessageEnvelope) {
        if self.active_edit_file_call.is_some()
            && self.approver_scopes.contains(&message.from_scope)
            && let Some(approver_response) = Self::parse_as::<ApprovalResponse>(&message)
        {
            self.active_edit_file_call
                .as_mut()
                .unwrap()
                .approver_responses
                .insert(message.from_scope.clone(), Some(approver_response));

            if self
                .active_edit_file_call
                .as_ref()
                .unwrap()
                .approver_responses
                .values()
                .all(|v| v.is_some())
            {
                let mut active_edit_file_call = self.active_edit_file_call.take().unwrap();

                let problems = active_edit_file_call
                    .approver_responses
                    .values_mut()
                    .filter_map(|v| match v.take().unwrap() {
                        ApprovalResponse::Approved => None,
                        ApprovalResponse::RequestChanges { changes } => Some(changes),
                    })
                    .collect::<Vec<String>>();
                if problems.len() > 0 {
                    let tool_response_message = format!(
                        "Your edit file request has been denied! Domain experts have requested the following changes:\n\n{}",
                        problems.join("\n\n--------\n\n")
                    );
                    self.send_error_result(
                        &active_edit_file_call.tool_call_id,
                        tool_response_message.clone(),
                        UIDisplayInfo {
                            collapsed: "Edit file request denied".to_string(),
                            expanded: Some(tool_response_message),
                        },
                    );
                } else {
                    self.do_edit_file(
                        &active_edit_file_call.tool_call_id,
                        &active_edit_file_call.edit_file_params,
                    );
                }
            }
        }

        if message.from_scope != self.scope {
            return;
        }

        if let Some(execute_tool) = Self::parse_as::<ExecuteTool>(&message) {
            match execute_tool.tool_call.function.name.as_str() {
                READ_FILE_NAME => self.handle_read_file(execute_tool),
                EDIT_FILE_NAME => self.handle_edit_file(execute_tool),
                _ => (),
            }
        }
    }

    fn destructor(&mut self) {}
}

impl FileInteractionWIthApprovalActor {
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
                        format!("Failed to parse edit_file parameters: {e}"),
                        UIDisplayInfo {
                            collapsed: "Parameters: Invalid format".to_string(),
                            expanded: Some(format!(
                                "Error: Failed to parse parameters\n\nDetails: {e}",
                            )),
                        },
                    );
                    return;
                }
            };

        let diff = match self.manager.get_edit_diff(&params) {
            Ok(diff) => diff,
            Err(e) => {
                self.send_error_result(
                    tool_call_id,
                    e.clone(),
                    UIDisplayInfo {
                        collapsed: e,
                        expanded: None,
                    },
                );
                return;
            }
        };

        self.active_edit_file_call = Some(ActiveEditFileCall {
            tool_call_id: execute_tool.tool_call.id,
            edit_file_params: params.clone(),
            approver_responses: self
                .approver_scopes
                .iter()
                .map(|x| (x.clone(), None))
                .collect(),
        });

        let message_content = format!("Review the change:\n\n{diff}");
        for scope in &self.approver_scopes {
            let message = ChatMessage::System(SystemChatMessage {
                content: message_content.clone(),
            });
            let _ = Self::broadcast_common_message(AddMessage {
                agent: scope.clone(),
                message,
            });
        }
    }

    fn do_edit_file(&mut self, tool_call_id: &str, params: &EditFileParams) {
        match self.manager.edit_file(params) {
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

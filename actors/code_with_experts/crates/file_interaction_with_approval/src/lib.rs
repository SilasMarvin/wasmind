use std::collections::HashMap;

use bindings::{
    exports::wasmind::actor::actor::MessageEnvelope,
    wasmind::actor::{actor::Scope, agent::spawn_agent},
};
use code_with_experts_common::ApprovalResponse;
use file_interaction::{
    EDIT_FILE_DESCRIPTION, EDIT_FILE_NAME, EDIT_FILE_SCHEMA, EditFileParams,
    FILE_TOOLS_USAGE_GUIDE, FileInteractionManager, READ_FILE_DESCRIPTION, READ_FILE_NAME,
    READ_FILE_SCHEMA, ReadFileParams,
};
use serde::Deserialize;
use wasmind_actor_utils::{
    common_messages::{
        assistant::{AddMessage, Section, SystemPromptContent, SystemPromptContribution},
        tools::{
            ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, ToolsAvailable,
            UIDisplayInfo,
        },
    },
    llm_client_types::{ChatMessage, SystemChatMessage},
};

#[allow(warnings)]
mod bindings;

#[derive(Deserialize)]
struct ApprovalConfig {
    approvers: HashMap<String, Vec<String>>,
}

wasmind_actor_utils::actors::macros::generate_actor_trait!();

struct ActiveEditFileCall {
    approver_scopes: Vec<Scope>,
    tool_call_id: String,
    originating_request_id: String,
    edit_file_params: EditFileParams,
    approver_responses: HashMap<Scope, Option<ApprovalResponse>>,
}

#[derive(wasmind_actor_utils::actors::macros::Actor)]
pub struct FileInteractionWIthApprovalActor {
    scope: String,
    manager: FileInteractionManager,
    active_edit_file_call: Option<ActiveEditFileCall>,
    config: ApprovalConfig,
}

impl GeneratedActorTrait for FileInteractionWIthApprovalActor {
    fn new(scope: String, config_str: String) -> Self {
        let config: ApprovalConfig =
            toml::from_str(&config_str).expect("Error deserializing config");

        let tools = vec![
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
            config,
            scope: scope.clone(),
            manager: FileInteractionManager::new(),
            active_edit_file_call: None,
        }
    }

    fn handle_message(&mut self, message: MessageEnvelope) {
        if self.active_edit_file_call.is_some()
            && self
                .active_edit_file_call
                .as_ref()
                .unwrap()
                .approver_scopes
                .contains(&message.from_scope)
            && let Some(approver_response) = Self::parse_as::<ApprovalResponse>(&message)
        {
            self.active_edit_file_call
                .as_mut()
                .unwrap()
                .approver_responses
                .insert(message.from_scope.clone(), Some(approver_response));

            self.update_tool_call_status();

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
                let requested_changes = if problems.len() > 0 {
                    Some(problems.join("\n\n--------\n\n"))
                } else {
                    None
                };
                self.do_edit_file(
                    &active_edit_file_call.tool_call_id,
                    &active_edit_file_call.originating_request_id,
                    &active_edit_file_call.edit_file_params,
                    requested_changes,
                );
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

        let default_template = r#"The current state of all read and edited files. This is updated automatically for you after each edit_file and read_file call. I.E. You do NOT need to call read_file after edit_file uses

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

        let params: ReadFileParams =
            match serde_json::from_str(&execute_tool.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    self.send_error_result(
                        tool_call_id,
                        &execute_tool.originating_request_id,
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
                let message = format!(
                    "{} -- Check the FilesReadAndEdited section in the SystemPrompt to see the read file",
                    result.message
                );
                self.send_success_result(
                    tool_call_id,
                    &execute_tool.originating_request_id,
                    message,
                    result.ui_display,
                );
            }
            Err(error) => {
                self.send_error_result(
                    tool_call_id,
                    &execute_tool.originating_request_id,
                    error.error_msg,
                    error.ui_display,
                );
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
                        &execute_tool.originating_request_id,
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
                    &execute_tool.originating_request_id,
                    e.clone(),
                    UIDisplayInfo {
                        collapsed: e,
                        expanded: None,
                    },
                );
                return;
            }
        };

        let approver_scopes: Vec<Scope> = self
            .config
            .approvers
            .iter()
            .map(|(approver_name, approver_actors)| {
                let mut approver_actors = approver_actors.clone();
                approver_actors.extend_from_slice(&[
                    "hcwe_approve".to_string(),
                    "hcwe_request_changes".to_string(),
                ]);
                spawn_agent(&approver_actors, &approver_name)
                    .expect("Error spawning initial actors")
            })
            .collect();

        let message_content = format!("Review the change:\n\n{diff}");
        for scope in &approver_scopes {
            let message = ChatMessage::System(SystemChatMessage {
                content: message_content.clone(),
            });
            let _ = Self::broadcast_common_message(AddMessage {
                agent: scope.clone(),
                message,
            });
        }

        self.active_edit_file_call = Some(ActiveEditFileCall {
            tool_call_id: execute_tool.tool_call.id,
            originating_request_id: execute_tool.originating_request_id,
            edit_file_params: params.clone(),
            approver_responses: approver_scopes.iter().map(|x| (x.clone(), None)).collect(),
            approver_scopes,
        });

        self.update_tool_call_status();
    }

    fn update_tool_call_status(&self) {
        if let Some(active_edit_file_call) = &self.active_edit_file_call {
            let _ = Self::broadcast_common_message(ToolCallStatusUpdate {
                id: active_edit_file_call.tool_call_id.clone(),
                originating_request_id: active_edit_file_call.originating_request_id.clone(),
                status: ToolCallStatus::Received {
                    display_info: UIDisplayInfo {
                        collapsed: format!(
                            "Waiting for {}/{} experts",
                            active_edit_file_call
                                .approver_responses
                                .values()
                                .filter(|x| x.is_some())
                                .count(),
                            active_edit_file_call.approver_scopes.len()
                        ),
                        expanded: None,
                    },
                },
            });
        }
    }

    fn do_edit_file(
        &mut self,
        tool_call_id: &str,
        originating_request_id: &str,
        params: &EditFileParams,
        requested_changes: Option<String>,
    ) {
        match self.manager.edit_file(params) {
            Ok(result) => {
                self.update_unified_files_system_prompt();
                let message = format!(
                    "{} -- Check the FilesReadAndEdited section in the SystemPrompt to see the updated edited file",
                    result.message
                );
                let message = if let Some(requested_changes) = requested_changes {
                    format!(
                        "{message}\n\nExperts have also reviewed your changes and requested you make the following changes:\n{requested_changes}"
                    )
                } else {
                    format!(
                        "{message}\n\nExperts have also reviewed your changes and approved them! Excellent job!"
                    )
                };
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

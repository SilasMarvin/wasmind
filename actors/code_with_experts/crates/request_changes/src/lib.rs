use bindings::exports::wasmind::actor::actor::MessageEnvelope;
use code_with_experts_common::ApprovalResponse;
use wasmind_actor_utils::common_messages::{
    assistant::{
        AgentTaskResponse, RequestStatusUpdate, Section, Status, SystemPromptContent,
        SystemPromptContribution, WaitReason,
    },
    tools::{
        ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, ToolsAvailable,
        UIDisplayInfo,
    },
};
use serde::Deserialize;

#[allow(warnings)]
mod bindings;

wasmind_actor_utils::actors::macros::generate_actor_trait!();

const REQUEST_CHANGES_NAME: &str = "request_changes";
const REQUEST_CHANGES_DESCRIPTION: &str = "Request changes to the proposed file edits";
const REQUEST_CHANGES_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "changes_requested": {
            "type": "string",
            "description": "Clear description of what changes are needed"
        }
    },
    "required": ["changes_requested"]
}"#;

#[derive(Deserialize)]
struct RequestChangesParams {
    changes_requested: String,
}

#[derive(wasmind_actor_utils::actors::macros::Actor)]
pub struct RequestChangesActor {
    scope: String,
}

impl GeneratedActorTrait for RequestChangesActor {
    fn new(scope: String, _config_str: String) -> Self {
        let tools = vec![wasmind_actor_utils::llm_client_types::Tool {
            tool_type: "function".to_string(),
            function: wasmind_actor_utils::llm_client_types::ToolFunctionDefinition {
                name: REQUEST_CHANGES_NAME.to_string(),
                description: REQUEST_CHANGES_DESCRIPTION.to_string(),
                parameters: serde_json::from_str(REQUEST_CHANGES_SCHEMA).unwrap(),
            },
        }];

        let _ = Self::broadcast_common_message(ToolsAvailable { tools });

        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "request_changes:usage".to_string(),
            content: SystemPromptContent::Text(
                "Use the request_changes tool when you need changes. Be very clear about what you are requesting they change.".to_string(),
            ),
            priority: 900,
            section: Some(Section::Tools),
        });

        Self { scope }
    }

    fn handle_message(&mut self, message: MessageEnvelope) {
        if message.from_scope != self.scope {
            return;
        }

        if let Some(execute_tool) = Self::parse_as::<ExecuteTool>(&message) {
            match execute_tool.tool_call.function.name.as_str() {
                REQUEST_CHANGES_NAME => self.handle_request_changes(execute_tool),
                _ => {}
            }
        }
    }

    fn destructor(&mut self) {}
}

impl RequestChangesActor {
    fn handle_request_changes(&self, execute_tool: ExecuteTool) {
        let tool_call_id = &execute_tool.tool_call.id;

        let params: RequestChangesParams =
            match serde_json::from_str(&execute_tool.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    let update = ToolCallStatusUpdate {
                        id: tool_call_id.to_string(),
                        originating_request_id: execute_tool.originating_request_id.clone(),
                        status: ToolCallStatus::Done {
                            result: Err(ToolCallResult {
                                content: format!(
                                    "Failed to parse request_changes parameters: {}",
                                    e
                                ),
                                ui_display_info: UIDisplayInfo {
                                    collapsed: "Parameters: Invalid format".to_string(),
                                    expanded: Some(format!(
                                        "Error: Failed to parse parameters\n\nDetails: {}",
                                        e
                                    )),
                                },
                            }),
                        },
                    };
                    let _ = Self::broadcast_common_message(update);
                    return;
                }
            };

        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: execute_tool.originating_request_id.clone(),
            status: ToolCallStatus::Done {
                result: Ok(ToolCallResult {
                    content: format!("Changes requested: {}", params.changes_requested),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "Changes requested".to_string(),
                        expanded: Some(params.changes_requested.clone()),
                    },
                }),
            },
        };

        let _ = Self::broadcast_common_message(ApprovalResponse::RequestChanges {
            changes: params.changes_requested,
        });

        let _ = Self::broadcast_common_message(RequestStatusUpdate {
            agent: self.scope.clone(),
            status: Status::Done {
                result: Ok(AgentTaskResponse {
                    summary: "Requested changes".to_string(),
                    success: true,
                }),
            },
            originating_request_id: Some(execute_tool.originating_request_id.clone()),
        });

        let _ = Self::broadcast_common_message(update);
    }
}

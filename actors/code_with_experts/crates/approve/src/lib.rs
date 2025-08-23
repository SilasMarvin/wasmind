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

#[allow(warnings)]
mod bindings;

wasmind_actor_utils::actors::macros::generate_actor_trait!();

const APPROVE_NAME: &str = "approve";
const APPROVE_DESCRIPTION: &str = "Approve the proposed file changes";
const APPROVE_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {},
    "required": []
}"#;

#[derive(wasmind_actor_utils::actors::macros::Actor)]
pub struct ApproveActor {
    scope: String,
}

impl GeneratedActorTrait for ApproveActor {
    fn new(scope: String, _config_str: String) -> Self {
        let tools = vec![wasmind_actor_utils::llm_client_types::Tool {
            tool_type: "function".to_string(),
            function: wasmind_actor_utils::llm_client_types::ToolFunctionDefinition {
                name: APPROVE_NAME.to_string(),
                description: APPROVE_DESCRIPTION.to_string(),
                parameters: serde_json::from_str(APPROVE_SCHEMA).unwrap(),
            },
        }];

        let _ = Self::broadcast_common_message(ToolsAvailable { tools });

        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "approve:usage".to_string(),
            content: SystemPromptContent::Text(
                "Use the approve tool when you approve the proposed file changes.".to_string(),
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
                APPROVE_NAME => self.handle_approve(execute_tool),
                _ => {}
            }
        }
    }

    fn destructor(&mut self) {}
}

impl ApproveActor {
    fn handle_approve(&self, execute_tool: ExecuteTool) {
        let tool_call_id = &execute_tool.tool_call.id;

        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: execute_tool.originating_request_id.clone(),
            status: ToolCallStatus::Done {
                result: Ok(ToolCallResult {
                    content: "Changes approved".to_string(),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "Approved".to_string(),
                        expanded: Some("File changes have been approved".to_string()),
                    },
                }),
            },
        };

        let _ = Self::broadcast_common_message(ApprovalResponse::Approved);

        let _ = Self::broadcast_common_message(RequestStatusUpdate {
            agent: self.scope.clone(),
            status: Status::Done {
                result: Ok(AgentTaskResponse {
                    summary: "Approved edit_file request".to_string(),
                    success: true,
                }),
            },
            originating_request_id: Some(execute_tool.originating_request_id.clone()),
        });

        let _ = Self::broadcast_common_message(update);
    }
}

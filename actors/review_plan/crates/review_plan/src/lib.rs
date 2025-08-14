use bindings::exports::wasmind::actor::actor::MessageEnvelope;
use review_plan_common::PlanReviewResponse;
use wasmind_actor_utils::common_messages::{
    assistant::{
        AgentTaskResponse, RequestStatusUpdate, Section, Status, SystemPromptContent,
        SystemPromptContribution,
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

const REVIEW_PLAN_NAME: &str = "review_plan";
const REVIEW_PLAN_DESCRIPTION: &str = "Provide feedback on the submitted plan";
const REVIEW_PLAN_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "feedback": {
            "type": "string",
            "description": "Detailed feedback on the plan - what works well, potential issues, suggestions for improvement"
        }
    },
    "required": ["feedback"]
}"#;

#[derive(Deserialize)]
struct ReviewPlanParams {
    feedback: String,
}

#[derive(wasmind_actor_utils::actors::macros::Actor)]
pub struct ReviewPlanActor {
    scope: String,
}

impl GeneratedActorTrait for ReviewPlanActor {
    fn new(scope: String, _config_str: String) -> Self {
        let tools = vec![wasmind_actor_utils::llm_client_types::Tool {
            tool_type: "function".to_string(),
            function: wasmind_actor_utils::llm_client_types::ToolFunctionDefinition {
                name: REVIEW_PLAN_NAME.to_string(),
                description: REVIEW_PLAN_DESCRIPTION.to_string(),
                parameters: serde_json::from_str(REVIEW_PLAN_SCHEMA).unwrap(),
            },
        }];

        let _ = Self::broadcast_common_message(ToolsAvailable { tools });

        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "review_plan:usage".to_string(),
            content: SystemPromptContent::Text(
                "Use the review_plan tool to provide feedback on the submitted plan. You must provide detailed, constructive feedback.".to_string(),
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
                REVIEW_PLAN_NAME => self.handle_review_plan(execute_tool),
                _ => {}
            }
        }
    }

    fn destructor(&mut self) {}
}

impl ReviewPlanActor {
    fn handle_review_plan(&self, execute_tool: ExecuteTool) {
        let tool_call_id = &execute_tool.tool_call.id;

        let params: ReviewPlanParams =
            match serde_json::from_str(&execute_tool.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    let update = ToolCallStatusUpdate {
                        id: tool_call_id.to_string(),
                        originating_request_id: execute_tool.originating_request_id.clone(),
                        status: ToolCallStatus::Done {
                            result: Err(ToolCallResult {
                                content: format!(
                                    "Failed to parse review_plan parameters: {}",
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
                    content: format!("Plan feedback provided: {}", params.feedback),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "Feedback provided".to_string(),
                        expanded: Some(params.feedback.clone()),
                    },
                }),
            },
        };

        let _ = Self::broadcast_common_message(PlanReviewResponse {
            feedback: params.feedback,
        });

        let _ = Self::broadcast_common_message(RequestStatusUpdate {
            agent: self.scope.clone(),
            status: Status::Done {
                result: Ok(AgentTaskResponse {
                    summary: "Provided plan feedback".to_string(),
                    success: true,
                }),
            },
            originating_request_id: Some(execute_tool.originating_request_id.clone()),
        });

        let _ = Self::broadcast_common_message(update);
    }
}
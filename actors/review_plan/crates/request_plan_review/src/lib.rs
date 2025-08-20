use std::collections::HashMap;

use bindings::{
    exports::wasmind::actor::actor::MessageEnvelope,
    wasmind::actor::{actor::Scope, agent::spawn_agent},
};
use review_plan_common::PlanReviewResponse;
use serde::Deserialize;
use wasmind_actor_utils::{
    common_messages::{
        assistant::{AddMessage, Section, SystemPromptContent, SystemPromptContribution},
        tools::{
            ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, ToolsAvailable,
            UIDisplayInfo,
        },
    },
    llm_client_types::{ChatMessage, UserChatMessage},
};

#[allow(warnings)]
mod bindings;

#[derive(Deserialize)]
struct ReviewConfig {
    reviewers: HashMap<String, Vec<String>>,
}

wasmind_actor_utils::actors::macros::generate_actor_trait!();

const REQUEST_PLAN_REVIEW_USAGE_GUIDE: &str = r#"<tool name="request_plan_review">Get Expert Review Before Execution

**CRITICAL: Always use this tool BEFORE creating your final plan and executing tasks!**

**Purpose**: Submit your task description and proposed plan to expert reviewers who will provide feedback, catch potential issues, and suggest improvements before you begin execution.

**When to Use (MANDATORY)**:
- ✅ **ALWAYS before major task execution** - This should be your first step after understanding the user's request
- ✅ For complex multi-step plans that involve multiple agents or tools
- ✅ When working with unfamiliar technologies or domains
- ✅ Before making significant changes to systems or codebases
- ✅ When the stakes are high and errors would be costly

**When NOT to Use**:
- ❌ For trivial single-step tasks (like reading one file)
- ❌ After you've already started executing (get review first!)
- ❌ For minor corrections or tiny adjustments

**How to Use Effectively**:

**Task Description**:
- Be specific about what you're trying to accomplish
- Include context about the user's goals and constraints
- Mention any relevant background information
- Example: "User wants to set up a CI/CD pipeline for their React app that runs tests and deploys to AWS S3 on every push to main branch"

**Plan Description**:
- Break down your approach into clear steps
- Explain your reasoning for major decisions
- Identify potential risks or challenges you foresee
- Include what tools/agents you plan to use
- Example: "1. Create GitHub Actions workflow file, 2. Configure AWS credentials, 3. Set up S3 bucket with static hosting, 4. Create deployment script, 5. Test the pipeline"

**Interpreting Feedback**:
- Take all feedback seriously - reviewers are domain experts
- If multiple reviewers agree on an issue, definitely address it
- Ask for clarification if feedback is unclear
- Revise your plan based on feedback before proceeding

**Best Practice Workflow**:
1. Understand user request
2. **Use request_plan_review** with task + initial plan
3. Receive and analyze expert feedback
4. Revise plan based on feedback
5. Begin execution with confidence

Remember: A few minutes of review can save hours of fixing mistakes!
</tool>"#;

const REQUEST_PLAN_REVIEW_NAME: &str = "request_plan_review";
const REQUEST_PLAN_REVIEW_DESCRIPTION: &str =
    "Submit a task and plan for review by expert reviewers before execution";
const REQUEST_PLAN_REVIEW_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "task": {
            "type": "string",
            "description": "Description of what you are trying to accomplish"
        },
        "plan": {
            "type": "string",
            "description": "Your proposed plan for accomplishing the task"
        }
    },
    "required": ["task", "plan"]
}"#;

#[derive(Deserialize)]
struct RequestPlanReviewParams {
    task: String,
    plan: String,
}

struct ActivePlanReview {
    reviewer_scopes: Vec<Scope>,
    tool_call_id: String,
    originating_request_id: String,
    task: String,
    plan: String,
    reviewer_responses: HashMap<Scope, Option<PlanReviewResponse>>,
}

#[derive(wasmind_actor_utils::actors::macros::Actor)]
pub struct RequestPlanReviewActor {
    scope: String,
    active_plan_review: Option<ActivePlanReview>,
    config: ReviewConfig,
}

impl GeneratedActorTrait for RequestPlanReviewActor {
    fn new(scope: String, config_str: String) -> Self {
        let config: ReviewConfig = toml::from_str(&config_str).expect("Error deserializing config");

        let tools = vec![wasmind_actor_utils::llm_client_types::Tool {
            tool_type: "function".to_string(),
            function: wasmind_actor_utils::llm_client_types::ToolFunctionDefinition {
                name: REQUEST_PLAN_REVIEW_NAME.to_string(),
                description: REQUEST_PLAN_REVIEW_DESCRIPTION.to_string(),
                parameters: serde_json::from_str(REQUEST_PLAN_REVIEW_SCHEMA).unwrap(),
            },
        }];

        let _ = Self::broadcast_common_message(ToolsAvailable { tools });

        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "request_plan_review:usage_guide".to_string(),
            content: SystemPromptContent::Text(REQUEST_PLAN_REVIEW_USAGE_GUIDE.to_string()),
            priority: 900,
            section: Some(Section::Tools),
        });

        Self {
            scope,
            active_plan_review: None,
            config,
        }
    }

    fn handle_message(&mut self, message: MessageEnvelope) {
        if self.active_plan_review.is_some()
            && self
                .active_plan_review
                .as_ref()
                .unwrap()
                .reviewer_scopes
                .contains(&message.from_scope)
        {
            if let Some(plan_review_response) = Self::parse_as::<PlanReviewResponse>(&message) {
                self.active_plan_review
                    .as_mut()
                    .unwrap()
                    .reviewer_responses
                    .insert(message.from_scope.clone(), Some(plan_review_response));

                self.update_tool_call_status();

                if self
                    .active_plan_review
                    .as_ref()
                    .unwrap()
                    .reviewer_responses
                    .values()
                    .all(|v| v.is_some())
                {
                    let active_plan_review = self.active_plan_review.take().unwrap();
                    self.complete_plan_review(active_plan_review);
                }
            }
        }

        if message.from_scope != self.scope {
            return;
        }

        if let Some(execute_tool) = Self::parse_as::<ExecuteTool>(&message) {
            match execute_tool.tool_call.function.name.as_str() {
                REQUEST_PLAN_REVIEW_NAME => self.handle_request_plan_review(execute_tool),
                _ => {}
            }
        }
    }

    fn destructor(&mut self) {}
}

impl RequestPlanReviewActor {
    fn handle_request_plan_review(&mut self, execute_tool: ExecuteTool) {
        let tool_call_id = &execute_tool.tool_call.id;

        let params: RequestPlanReviewParams =
            match serde_json::from_str(&execute_tool.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    self.send_error_result(
                        tool_call_id,
                        &execute_tool.originating_request_id,
                        format!("Failed to parse request_plan_review parameters: {}", e),
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

        let reviewer_scopes: Vec<Scope> = self
            .config
            .reviewers
            .iter()
            .map(|(reviewer_name, reviewer_actors)| {
                let mut reviewer_actors = reviewer_actors.clone();
                reviewer_actors.push("rpr__review_plan".to_string());
                spawn_agent(&reviewer_actors, &reviewer_name)
                    .expect("Error spawning reviewer agents")
            })
            .collect();

        let message_content = format!(
            "Review this plan for the following task:\n\nTask: {}\n\nPlan: {}",
            params.task, params.plan
        );
        for scope in &reviewer_scopes {
            let message = ChatMessage::User(UserChatMessage {
                content: message_content.clone(),
            });
            let _ = Self::broadcast_common_message(AddMessage {
                agent: scope.clone(),
                message,
            });
        }

        self.active_plan_review = Some(ActivePlanReview {
            tool_call_id: execute_tool.tool_call.id,
            originating_request_id: execute_tool.originating_request_id,
            task: params.task,
            plan: params.plan,
            reviewer_responses: reviewer_scopes.iter().map(|x| (x.clone(), None)).collect(),
            reviewer_scopes,
        });

        self.update_tool_call_status();
    }

    fn complete_plan_review(&self, active_plan_review: ActivePlanReview) {
        let feedback: Vec<String> = active_plan_review
            .reviewer_responses
            .values()
            .filter_map(|response| response.as_ref().map(|r| r.feedback.clone()))
            .collect();

        let combined_feedback = feedback.join("\n\n--------\n\n");

        let update = ToolCallStatusUpdate {
            id: active_plan_review.tool_call_id.clone(),
            originating_request_id: active_plan_review.originating_request_id.clone(),
            status: ToolCallStatus::Done {
                result: Ok(ToolCallResult {
                    content: format!("Plan review completed. Feedback:\n\n{}", combined_feedback),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "Plan review completed".to_string(),
                        expanded: Some(combined_feedback),
                    },
                }),
            },
        };
        let _ = Self::broadcast_common_message(update);
    }

    fn update_tool_call_status(&self) {
        if let Some(active_plan_review) = &self.active_plan_review {
            let completed_reviews = active_plan_review
                .reviewer_responses
                .values()
                .filter(|v| v.is_some())
                .count();
            let total_reviews = active_plan_review.reviewer_responses.len();

            let _ = Self::broadcast_common_message(ToolCallStatusUpdate {
                id: active_plan_review.tool_call_id.clone(),
                originating_request_id: active_plan_review.originating_request_id.clone(),
                status: ToolCallStatus::Received {
                    display_info: UIDisplayInfo {
                        collapsed: format!(
                            "Waiting for reviews: {}/{}",
                            completed_reviews, total_reviews
                        ),
                        expanded: None,
                    },
                },
            });
        }
    }

    fn send_error_result(
        &self,
        tool_call_id: &str,
        originating_request_id: &str,
        content: String,
        ui_display_info: UIDisplayInfo,
    ) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            originating_request_id: originating_request_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(ToolCallResult {
                    content,
                    ui_display_info,
                }),
            },
        };
        let _ = Self::broadcast_common_message(update);
    }
}

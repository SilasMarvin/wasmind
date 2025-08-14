use wasmind_actor_utils::common_messages::Message;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanReviewResponse {
    pub feedback: String,
}

impl Message for PlanReviewResponse {
    const MESSAGE_TYPE: &str = "review_plan.common.PlanReviewResponse";
}
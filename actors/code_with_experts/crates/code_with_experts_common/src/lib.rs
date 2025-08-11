use wasmind_actor_utils::common_messages::Message;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApprovalResponse {
    Approved,
    RequestChanges { changes: String },
}

impl Message for ApprovalResponse {
    const MESSAGE_TYPE: &str = "code_with_experts.common.ApprovalResponse";
}
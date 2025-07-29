use serde::{Serialize, de::DeserializeOwned};

pub trait Message: Serialize + DeserializeOwned {
    const MESSAGE_TYPE: &str;
}

pub type Scope = String;

pub mod actors {
    use super::Message;
    use serde::{Deserialize, Serialize};

    // hive.common.actors.ActorReady
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ActorReady;

    impl Message for ActorReady {
        const MESSAGE_TYPE: &str = "hive.common.actors.ActorReady";
    }

    // hive.common.actors.Exit
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Exit;

    impl Message for Exit {
        const MESSAGE_TYPE: &str = "hive.common.actors.Exit";
    }
}

pub mod assistant {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use uuid::Uuid;

    use crate::Scope;

    use super::Message;
    use super::tools;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PendingToolCall {
        pub tool_call: hive_llm_types::types::ToolCall,
        pub result: Option<Result<tools::ToolCallResult, tools::ToolCallResult>>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum WaitReason {
        WaitingForUserInput,
        WaitingForSystemOrUser {
            tool_name: Option<String>,
            tool_call_id: String,
            required_scope_id: Option<String>,
        },
        WaitingForTools {
            tool_calls: HashMap<String, PendingToolCall>,
        },
        WaitingForLiteLLM,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AgentTaskResponse {
        pub summary: String,
        pub success: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum Status {
        Processing {
            request_id: Uuid,
        },
        Wait {
            reason: WaitReason,
        },
        Done {
            result: Result<AgentTaskResponse, String>,
        },
    }

    // hive.common.assistant.StatusUpdate
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct StatusUpdate {
        pub status: Status,
    }

    impl Message for StatusUpdate {
        const MESSAGE_TYPE: &str = "hive.common.assistant.StatusUpdate";
    }

    // hive.common.assistant.RequestStatusUpdate
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct RequestStatusUpdate {
        pub agent: Scope,
        pub status: Status,
        pub tool_call_id: Option<String>,
    }

    impl Message for RequestStatusUpdate {
        const MESSAGE_TYPE: &str = "hive.common.assistant.RequestStatusUpdate";
    }

    // hive.common.assistant.InterruptAndForceWaitForSystem
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct InterruptAndForceWaitForSystem {
        pub agent: Scope,
        pub required_scope: Option<Scope>,
    }

    impl Message for InterruptAndForceWaitForSystem {
        const MESSAGE_TYPE: &str = "hive.common.assistant.InterruptAndForceWaitForSystem";
    }

    // hive.common.assistant.AddMessage
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AddMessage {
        pub agent: Scope,
        pub message: hive_llm_types::types::ChatMessage,
    }

    impl Message for AddMessage {
        const MESSAGE_TYPE: &str = "hive.common.assistant.AddMessage";
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ChatState {
        pub system: hive_llm_types::types::SystemChatMessage,
        pub tools: Vec<hive_llm_types::types::Tool>,
        pub messages: Vec<hive_llm_types::types::ChatMessage>,
    }

    // hive.common.assistant.Request
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Request {
        chat_state: ChatState,
    }

    impl Message for Request {
        const MESSAGE_TYPE: &str = "hive.common.assistant.Request";
    }

    // hive.common.assistant.Response
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Response {
        pub request_id: Uuid,
        pub message: hive_llm_types::types::AssistantChatMessage,
    }

    impl Message for Response {
        const MESSAGE_TYPE: &str = "hive.common.assistant.Response";
    }

    // hive.common.assistant.ChatStateUpdated
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ChatStateUpdated {
        chat_state: ChatState,
    }

    impl Message for ChatStateUpdated {
        const MESSAGE_TYPE: &str = "hive.common.assistant.ChatStateUpdated";
    }

    // System prompt contribution system
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum SystemPromptContent {
        /// Pre-rendered text that goes directly into the prompt
        Text(String),
        /// Structured data with a default template for rendering
        Data {
            data: serde_json::Value,
            default_template: String,
        },
    }

    // hive.common.assistant.SystemPromptContribution
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SystemPromptContribution {
        /// The agent (scope) this contribution is targeting
        pub agent: Scope,
        /// Unique key in format "actor_type.contribution_name" (e.g., "file_reader.open_files")
        pub key: String,
        /// The actual content to include in the system prompt
        pub content: SystemPromptContent,
        /// Priority for ordering within sections (higher = earlier)
        pub priority: i32,
        /// Optional section this belongs to (e.g., "context", "tools", "instructions")
        pub section: Option<String>,
    }

    impl Message for SystemPromptContribution {
        const MESSAGE_TYPE: &str = "hive.common.assistant.SystemPromptContribution";
    }
}

pub mod tools {
    use super::Message;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct UIDisplayInfo {
        pub collapsed: String,
        pub expanded: Option<String>,
    }

    // hive.common.tools.ToolsAvailable
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ToolsAvailable {
        pub tools: Vec<hive_llm_types::types::Tool>,
    }

    impl Message for ToolsAvailable {
        const MESSAGE_TYPE: &str = "hive.common.tools.ToolsAvailable";
    }

    // hive.common.tools.ExecuteToolCall
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ExecuteTool {
        pub tool_call: hive_llm_types::types::ToolCall,
    }

    impl Message for ExecuteTool {
        const MESSAGE_TYPE: &str = "hive.common.tools.ExecuteToolCall";
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ToolCallResult {
        pub content: String,
        pub ui_display_info: UIDisplayInfo,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AwaitingSystemDetails {
        pub required_scope: Option<String>,
        pub ui_display_info: UIDisplayInfo,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum ToolCallStatus {
        Received {
            display_info: UIDisplayInfo,
        },
        AwaitingSystem {
            details: AwaitingSystemDetails,
        },
        Done {
            result: Result<ToolCallResult, ToolCallResult>,
        },
    }

    // hive.common.tools.ToolCallStatusUpdate
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ToolCallStatusUpdate {
        pub status: ToolCallStatus,
        pub id: String,
    }

    impl Message for ToolCallStatusUpdate {
        const MESSAGE_TYPE: &str = "hive.common.tools.ToolCallStatusUpdate";
    }
}

pub mod litellm {
    use super::Message;
    use serde::{Deserialize, Serialize};

    // hive.common.litellm.BaseUrlUpdate
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct BaseUrlUpdate {
        pub base_url: String,
        pub models_available: Vec<String>,
    }

    impl Message for BaseUrlUpdate {
        const MESSAGE_TYPE: &str = "hive.common.litellm.BaseUrlUpdate";
    }
}

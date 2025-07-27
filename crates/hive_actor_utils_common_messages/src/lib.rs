use serde::{Serialize, de::DeserializeOwned};

pub trait CommonMessage: Serialize + DeserializeOwned {
    const MESSAGE_TYPE: &str;
}

pub type Scope = String;

pub mod actors {
    use super::CommonMessage;
    use serde::{Deserialize, Serialize};

    // hive.common.actors.ActorReady
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ActorReady;

    impl CommonMessage for ActorReady {
        const MESSAGE_TYPE: &str = "hive.common.actors.ActorReady";
    }

    // hive.common.actors.Exit
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Exit;

    impl CommonMessage for Exit {
        const MESSAGE_TYPE: &str = "hive.common.actors.Exit";
    }
}

pub mod assistant {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use uuid::Uuid;

    use crate::Scope;

    use super::CommonMessage;
    use super::tools;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PendingToolCall {
        pub tool_call: hive_llm_client::types::ToolCall,
        pub result: Option<tools::ToolCallResult>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum WaitReason {
        WaitingForInput,
        WaitForSystem {
            tool_name: Option<String>,
            tool_call_id: String,
        },
        WaitingForTools {
            tool_calls: HashMap<String, PendingToolCall>,
        },
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

    impl CommonMessage for StatusUpdate {
        const MESSAGE_TYPE: &str = "hive.common.assistant.StatusUpdate";
    }

    // hive.common.assistant.RequestStatusUpdate
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct RequestStatusUpdate {
        pub agent: Scope,
        pub status: Status,
    }

    impl CommonMessage for RequestStatusUpdate {
        const MESSAGE_TYPE: &str = "hive.common.assistant.RequestStatusUpdate";
    }

    // hive.common.assistant.InterruptAndForceWaitForSystem
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct InterruptAndForceWaitForSystem {
        pub agent: Scope,
        pub required_scope: Option<Scope>,
    }

    impl CommonMessage for InterruptAndForceWaitForSystem {
        const MESSAGE_TYPE: &str = "hive.common.assistant.InterruptAndForceWaitForSystem";
    }

    // hive.common.assistant.AddMessage
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AddMessage {
        pub agent: Scope,
        pub message: hive_llm_client::types::ChatMessage,
    }

    impl CommonMessage for AddMessage {
        const MESSAGE_TYPE: &str = "hive.common.assistant.AddMessage";
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ChatState {
        pub system: hive_llm_client::types::SystemChatMessage,
        pub tools: Vec<hive_llm_client::types::Tool>,
        pub messages: Vec<hive_llm_client::types::ChatMessage>,
    }

    // hive.common.assistant.Request
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Request {
        chat_state: ChatState,
    }

    impl CommonMessage for Request {
        const MESSAGE_TYPE: &str = "hive.common.assistant.Request";
    }

    // hive.common.assistant.Response
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Response {
        request_id: Uuid,
        message: hive_llm_client::types::AssistantChatMessage,
    }

    impl CommonMessage for Response {
        const MESSAGE_TYPE: &str = "hive.common.assistant.Response";
    }

    // hive.common.assistant.ChatStateUpdated
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ChatStateUpdated {
        chat_state: ChatState,
    }

    impl CommonMessage for ChatStateUpdated {
        const MESSAGE_TYPE: &str = "hive.common.assistant.ChatStateUpdated";
    }
}

pub mod tools {
    use super::CommonMessage;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct UIDisplayInfo {
        pub collapsed: String,
        pub expanded: Option<String>,
    }

    // hive.common.tools.ToolsAvailable
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ToolsAvailable {
        pub tools: Vec<hive_llm_client::types::Tool>,
    }

    impl CommonMessage for ToolsAvailable {
        const MESSAGE_TYPE: &str = "hive.common.tools.ToolsAvailable";
    }

    // hive.common.tools.ExecuteToolCall
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ExecuteTool {
        tool_call: hive_llm_client::types::ToolCall,
    }

    impl CommonMessage for ExecuteTool {
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

    // hive.common.tools.ToolStatusUpdate
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum ToolCallStatusUpdate {
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

    impl CommonMessage for ToolCallStatusUpdate {
        const MESSAGE_TYPE: &str = "hive.common.tools.ToolCallStatusUpdate";
    }
}

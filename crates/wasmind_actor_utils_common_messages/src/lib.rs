use serde::{Serialize, de::DeserializeOwned};

pub trait Message: Serialize + DeserializeOwned {
    const MESSAGE_TYPE: &str;
}

pub type Scope = String;

pub mod actors {
    use super::{Message, Scope};
    use serde::{Deserialize, Serialize};

    // wasmind.common.actors.ActorReady
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ActorReady;

    impl Message for ActorReady {
        const MESSAGE_TYPE: &str = "wasmind.common.actors.ActorReady";
    }

    // wasmind.common.actors.Exit
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Exit;

    impl Message for Exit {
        const MESSAGE_TYPE: &str = "wasmind.common.actors.Exit";
    }

    // wasmind.common.actors.AllActorsReady
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AllActorsReady;

    impl Message for AllActorsReady {
        const MESSAGE_TYPE: &str = "wasmind.common.actors.AllActorsReady";
    }

    // wasmind.common.actors.AgentSpawned
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AgentSpawned {
        pub agent_id: Scope,             // The scope
        pub name: String,                // "Root Agent", "Code Reviewer", etc.
        pub parent_agent: Option<Scope>, // Parent scope UUID if spawned
        pub actors: Vec<String>,         // ["assistant", "execute_bash"]
    }

    impl Message for AgentSpawned {
        const MESSAGE_TYPE: &str = "wasmind.common.actors.AgentSpawned";
    }
}

pub mod assistant {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use wasmind_llm_types::ChatMessageWithRequestId;

    use super::{Message, Scope, tools};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PendingToolCall {
        pub tool_call: wasmind_llm_types::ToolCall,
        pub result: Option<Result<tools::ToolCallResult, tools::ToolCallResult>>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum WaitReason {
        WaitingForAllActorsReady,
        WaitingForUserInput,
        WaitingForSystemInput {
            required_scope: Option<Scope>,
            interruptible_by_user: bool,
        },
        WaitingForAgentCoordination {
            originating_request_id: String,
            coordinating_tool_name: String,
            target_agent_scope: Option<Scope>,
            user_can_interrupt: bool,
        },
        WaitingForTools {
            originating_request_id: String,
            tool_calls: HashMap<String, PendingToolCall>,
        },
        WaitingForLiteLLM,
        CompactingConversation,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AgentTaskResponse {
        pub summary: String,
        pub success: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum Status {
        Processing {
            request_id: String,
        },
        Wait {
            reason: WaitReason,
        },
        Done {
            result: Result<AgentTaskResponse, String>,
        },
    }

    // wasmind.common.assistant.StatusUpdate
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct StatusUpdate {
        pub status: Status,
    }

    impl Message for StatusUpdate {
        const MESSAGE_TYPE: &str = "wasmind.common.assistant.StatusUpdate";
    }

    // wasmind.common.assistant.RequestStatusUpdate
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct RequestStatusUpdate {
        pub agent: Scope,
        pub status: Status,
        pub originating_request_id: Option<String>,
    }

    impl Message for RequestStatusUpdate {
        const MESSAGE_TYPE: &str = "wasmind.common.assistant.RequestStatusUpdate";
    }

    // wasmind.common.assistant.InterruptAndForceStatus
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct InterruptAndForceStatus {
        pub agent: Scope,
        pub status: Status,
    }

    impl Message for InterruptAndForceStatus {
        const MESSAGE_TYPE: &str = "wasmind.common.assistant.InterruptAndForceStatus";
    }

    // wasmind.common.assistant.AddMessage
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AddMessage {
        pub agent: Scope,
        pub message: wasmind_llm_types::ChatMessage,
    }

    impl Message for AddMessage {
        const MESSAGE_TYPE: &str = "wasmind.common.assistant.AddMessage";
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ChatState {
        pub system: wasmind_llm_types::SystemChatMessage,
        pub tools: Vec<wasmind_llm_types::Tool>,
        pub messages: Vec<wasmind_llm_types::ChatMessageWithRequestId>,
    }

    // wasmind.common.assistant.Request
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Request {
        pub chat_state: ChatState,
    }

    impl Message for Request {
        const MESSAGE_TYPE: &str = "wasmind.common.assistant.Request";
    }

    // wasmind.common.assistant.Response
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Response {
        pub message: wasmind_llm_types::AssistantChatMessageWithOriginatingRequestId,
        pub usage: wasmind_llm_types::Usage,
    }

    impl Message for Response {
        const MESSAGE_TYPE: &str = "wasmind.common.assistant.Response";
    }

    // wasmind.common.assistant.ChatStateUpdated
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ChatStateUpdated {
        pub chat_state: ChatState,
    }

    impl Message for ChatStateUpdated {
        const MESSAGE_TYPE: &str = "wasmind.common.assistant.ChatStateUpdated";
    }

    // System prompt section organization
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum Section {
        Identity,
        Context,
        Capabilities,
        Guidelines,
        Tools,
        Instructions,
        SystemContext,
        Custom(String),
    }

    impl Section {
        pub fn display_name(&self) -> String {
            match self {
                Section::Identity => "Identity".to_string(),
                Section::Context => "Context".to_string(),
                Section::Capabilities => "Capabilities".to_string(),
                Section::Guidelines => "Guidelines".to_string(),
                Section::Tools => "Tools".to_string(),
                Section::Instructions => "Instructions".to_string(),
                Section::SystemContext => "System Context".to_string(),
                Section::Custom(name) => name.clone(),
            }
        }
    }

    impl From<String> for Section {
        fn from(s: String) -> Self {
            match s.to_lowercase().as_str() {
                "identity" => Section::Identity,
                "context" => Section::Context,
                "capabilities" => Section::Capabilities,
                "guidelines" => Section::Guidelines,
                "tools" => Section::Tools,
                "instructions" => Section::Instructions,
                "system_context" | "system-context" => Section::SystemContext,
                _ => Section::Custom(s),
            }
        }
    }

    impl From<&str> for Section {
        fn from(s: &str) -> Self {
            Section::from(s.to_string())
        }
    }

    impl std::fmt::Display for Section {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let s = match self {
                Section::Identity => "identity",
                Section::Context => "context",
                Section::Capabilities => "capabilities",
                Section::Guidelines => "guidelines",
                Section::Tools => "tools",
                Section::Instructions => "instructions",
                Section::SystemContext => "system_context",
                Section::Custom(name) => name,
            };
            write!(f, "{s}")
        }
    }

    impl Serialize for Section {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            serializer.serialize_str(&self.to_string())
        }
    }

    impl<'de> Deserialize<'de> for Section {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let s = String::deserialize(deserializer)?;
            Ok(Section::from(s))
        }
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

    // wasmind.common.assistant.SystemPromptContribution
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
        /// Optional section this belongs to
        pub section: Option<Section>,
    }

    impl Message for SystemPromptContribution {
        const MESSAGE_TYPE: &str = "wasmind.common.assistant.SystemPromptContribution";
    }

    // wasmind.common.assistant.CompactedConversation
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CompactedConversation {
        pub agent: Scope,
        pub messages: Vec<ChatMessageWithRequestId>,
    }

    impl Message for CompactedConversation {
        const MESSAGE_TYPE: &str = "wasmind.common.assistant.CompactedConversation";
    }
}

pub mod tools {
    use super::{Message, Scope};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct UIDisplayInfo {
        pub collapsed: String,
        pub expanded: Option<String>,
    }

    // wasmind.common.tools.ToolsAvailable
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ToolsAvailable {
        pub tools: Vec<wasmind_llm_types::Tool>,
    }

    impl Message for ToolsAvailable {
        const MESSAGE_TYPE: &str = "wasmind.common.tools.ToolsAvailable";
    }

    // wasmind.common.tools.ExecuteToolCall
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ExecuteTool {
        pub tool_call: wasmind_llm_types::ToolCall,
        pub originating_request_id: String,
    }

    impl Message for ExecuteTool {
        const MESSAGE_TYPE: &str = "wasmind.common.tools.ExecuteToolCall";
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ToolCallResult {
        pub content: String,
        pub ui_display_info: UIDisplayInfo,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AwaitingSystemDetails {
        pub required_scope: Option<Scope>,
        pub ui_display_info: UIDisplayInfo,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum ToolCallStatus {
        Received {
            display_info: UIDisplayInfo,
        },
        // FUTURE NOTE: Is this one actually going to be useful?
        AwaitingSystem {
            details: AwaitingSystemDetails,
        },
        Done {
            result: Result<ToolCallResult, ToolCallResult>,
        },
    }

    // wasmind.common.tools.ToolCallStatusUpdate
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ToolCallStatusUpdate {
        pub status: ToolCallStatus,
        pub id: String,
        pub originating_request_id: String,
    }

    impl Message for ToolCallStatusUpdate {
        const MESSAGE_TYPE: &str = "wasmind.common.tools.ToolCallStatusUpdate";
    }
}

pub mod litellm {
    use super::Message;
    use serde::{Deserialize, Serialize};

    // wasmind.common.litellm.BaseUrlUpdate
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct BaseUrlUpdate {
        pub base_url: String,
        pub models_available: Vec<String>,
    }

    impl Message for BaseUrlUpdate {
        const MESSAGE_TYPE: &str = "wasmind.common.litellm.BaseUrlUpdate";
    }
}

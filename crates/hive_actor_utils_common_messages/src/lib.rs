pub trait CommonMessage {
    const MESSAGE_TYPE: &str;
}

pub mod actors {
    use super::CommonMessage;
    use serde::{Deserialize, Serialize};

    // hive.common.actors.ActorReady
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ActorReady {
        pub scope: String,
    }

    impl CommonMessage for ActorReady {
        const MESSAGE_TYPE: &str = "hive.common.actors.ActorReady";
    }
}

pub mod tools {
    use super::CommonMessage;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    // hive.common.tools.UIDisplayInfo
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct UIDisplayInfo {
        pub collapsed: String,
        pub expanded: Option<String>,
    }

    impl CommonMessage for UIDisplayInfo {
        const MESSAGE_TYPE: &str = "hive.common.tools.UIDisplayInfo";
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ToolDefinition {
        pub name: String,
        pub description: String,
        pub schema: Value,
    }

    // hive.common.tools.ToolsAvailable
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ToolsAvailable {
        pub tools: Vec<ToolDefinition>,
    }

    impl CommonMessage for ToolsAvailable {
        const MESSAGE_TYPE: &str = "hive.common.tools.ToolsAvailable";
    }

    // hive.common.tools.ToolCall
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ToolCall {
        pub id: String,
        pub name: String,
        pub arguments: Value,
    }

    impl CommonMessage for ToolCall {
        const MESSAGE_TYPE: &str = "hive.common.tools.ToolCall";
    }

    // hive.common.tools.ToolCallResult
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ToolCallResult {
        pub content: String,
        pub ui_display_info: UIDisplayInfo,
    }

    impl CommonMessage for ToolCallResult {
        const MESSAGE_TYPE: &str = "hive.common.tools.ToolCallResult";
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AwaitingSystemDetails {
        pub required_scope: Option<String>,
        pub ui_display_info: UIDisplayInfo,
    }

    // hive.common.tools.ToolStatusUpdate
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum ToolCallStatusUpdate {
        Received(UIDisplayInfo),
        AwaitingSystem(AwaitingSystemDetails),
        Done(Result<ToolCallResult, ToolCallResult>),
    }

    impl CommonMessage for ToolCallStatusUpdate {
        const MESSAGE_TYPE: &str = "hive.common.tools.ToolCallStatusUpdate";
    }
}

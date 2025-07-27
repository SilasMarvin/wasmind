use bindings::exports::hive::actor::actor::GuestActor;
use hive_actor_utils::{
    common_messages::{
        CommonMessage,
        assistant::{self, Status, WaitReason},
        tools::{self, ToolsAvailable},
    },
    llm_client_types::{self, ChatMessage},
};

#[allow(warnings)]
mod bindings;

/// Pending message that accumulates user input and system messages to be submitted to the LLM when appropriate
#[derive(Debug, Clone, Default)]
pub struct PendingMessage {
    /// Optional user content (only one at a time, new input replaces old)
    user_content: Option<String>,
    /// System messages that accumulate from sub-agents
    system_messages: Vec<String>,
}

impl PendingMessage {
    /// Create a new empty pending message
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if the pending message has any content
    pub fn has_content(&self) -> bool {
        self.user_content.is_some() || !self.system_messages.is_empty()
    }

    /// Add or replace user content
    pub fn set_user_content(&mut self, content: String) {
        self.user_content = Some(content);
    }

    /// Add a system message
    pub fn add_system_message(&mut self, message: String) {
        self.system_messages.push(message);
    }

    /// Convert to Vec<ChatMessage> for LLM submission
    /// System messages come first, then user message
    /// Returns empty vec if no content exists
    pub fn to_chat_messages(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::new();

        // Add system messages first
        for system_message in &self.system_messages {
            messages.push(ChatMessage::system(system_message.clone()));
        }

        // Add user message last if present
        if let Some(ref user_content) = self.user_content {
            messages.push(ChatMessage::user(user_content.clone()));
        }

        messages
    }

    /// Clear all content
    pub fn clear(&mut self) {
        self.user_content = None;
        self.system_messages.clear();
    }
}

#[derive(hive_actor_utils::actors::macros::Actor)]
pub struct Assistant {
    scope: String,
    chat_history: Vec<ChatMessage>,
    available_tools: Vec<llm_client_types::Tool>,
    status: Status,
}

hive_actor_utils::actors::macros::generate_actor_trait!();

impl GeneratedActorTrait for Assistant {
    fn new(scope: String) -> Self {
        Self {
            scope,
            chat_history: vec![],
            available_tools: vec![],
            status: Status::Wait {
                reason: WaitReason::WaitingForInput,
            },
        }
    }

    fn handle_message(
        &mut self,
        message: bindings::exports::hive::actor::actor::MessageEnvelope,
    ) -> () {
        if let Some(mut available_tools) =
            Self::parse_as::<tools::ToolsAvailable>(tools::ToolsAvailable::MESSAGE_TYPE, &message)
        {
            self.available_tools.append(&mut available_tools.tools);
        }
    }

    fn destructor(&mut self) -> () {
        todo!()
    }
}

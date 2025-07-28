use hive_actor_utils::{
    common_messages::{
        assistant::{self, Status, WaitReason},
        tools::{self, ToolCallStatus, ToolCallStatusUpdate},
    },
    llm_client_types::{self, ChatMessage, SystemChatMessage, UserChatMessage},
};

#[allow(warnings)]
mod bindings;

/// Pending message that accumulates user input and system messages to be submitted to the LLM when appropriate
#[derive(Debug, Clone, Default)]
pub struct PendingMessage {
    user_message: Option<UserChatMessage>,
    system_messages: Vec<SystemChatMessage>,
}

impl PendingMessage {
    /// Create a new empty pending message
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if the pending message has any content
    pub fn has_content(&self) -> bool {
        self.user_message.is_some() || !self.system_messages.is_empty()
    }

    /// Add or replace user content
    pub fn set_user_message(&mut self, message: UserChatMessage) {
        self.user_message = Some(message);
    }

    /// Add a system message
    pub fn add_system_message(&mut self, message: SystemChatMessage) {
        self.system_messages.push(message);
    }

    /// Convert to Vec<ChatMessage> for LLM submission
    /// System messages come first, then user message
    /// Returns empty vec if no content exists
    pub fn to_chat_messages(&mut self) -> Vec<ChatMessage> {
        let mut messages = Vec::new();

        // Add system messages first
        for system_message in self.system_messages.drain(..) {
            messages.push(ChatMessage::System(system_message));
        }

        // Add user message last if present
        if let Some(user_message) = self.user_message.take() {
            messages.push(ChatMessage::User(user_message));
        }

        messages
    }

    /// Clear all content
    pub fn clear(&mut self) {
        self.user_message = None;
        self.system_messages.clear();
    }
}

#[derive(hive_actor_utils::actors::macros::Actor)]
pub struct Assistant {
    pending_message: PendingMessage,
    scope: String,
    chat_history: Vec<ChatMessage>,
    available_tools: Vec<llm_client_types::Tool>,
    status: Status,
}

impl Assistant {
    fn add_chat_messages(&mut self, messages: impl IntoIterator<Item = ChatMessage>) {
        self.chat_history.extend(messages);

        // TODO: Broadcast the system state
        // let system_prompt = self.render_system_prompt();
        // self.broadcast(Message::AssistantChatUpdated(AssistantChatState {
        //     system: system_prompt,
        //     tools: self.available_tools.clone(),
        //     messages: self.chat_history.clone(),
        // }));
    }

    fn set_status(&mut self, new_status: Status, broadcast_change: bool) {
        self.status = new_status;

        if broadcast_change {
            // TODO: Broadcast state change
        }
    }

    fn submit(&mut self, submit_if_pending_message_is_empty: bool) {
        todo!()
    }

    fn handle_tool_call_update(&mut self, update: ToolCallStatusUpdate) {
        if let ToolCallStatus::Done { result, .. } = update.status {
            match &mut self.status.clone() {
                Status::Wait {
                    reason: WaitReason::WaitingForTools { tool_calls },
                } => {
                    let found = match tool_calls.get_mut(&update.id) {
                        Some(pending_call) => {
                            pending_call.result = Some(result);
                            true
                        }
                        None => false,
                    };

                    self.status = Status::Wait {
                        reason: WaitReason::WaitingForTools {
                            tool_calls: tool_calls.clone(),
                        },
                    };

                    if found
                        && tool_calls
                            .values()
                            .all(|pending_call| pending_call.result.is_some())
                    {
                        for (call_id, pending_call) in tool_calls.drain() {
                            let content = pending_call
                                .result
                                .unwrap()
                                .map(|tool_call_result| tool_call_result.content)
                                .unwrap_or_else(|e| format!("Error: {}", e.content));
                            self.add_chat_messages([ChatMessage::tool(
                                call_id,
                                pending_call.tool_call.function.name,
                                content,
                            )]);
                        }
                        self.submit(true);
                    }
                }
                Status::Wait {
                    reason:
                        WaitReason::WaitForSystem {
                            tool_name,
                            tool_call_id,
                            ..
                        },
                } => {
                    if tool_call_id != &update.id {
                        return;
                    }

                    let content = result
                        .map(|tool_call_result| tool_call_result.content)
                        .unwrap_or_else(|e| format!("Error: {}", e.content));

                    self.add_chat_messages([ChatMessage::tool(
                        update.id,
                        tool_name.clone().unwrap_or("system_tool".to_string()),
                        content,
                    )]);
                }
                _ => (),
            }
        }
    }
}

hive_actor_utils::actors::macros::generate_actor_trait!();

impl GeneratedActorTrait for Assistant {
    fn new(scope: String) -> Self {
        Self {
            scope,
            chat_history: vec![],
            available_tools: vec![],
            status: Status::Wait {
                reason: WaitReason::WaitingForUserInput,
            },
            pending_message: PendingMessage::new(),
        }
    }

    fn handle_message(
        &mut self,
        message: bindings::exports::hive::actor::actor::MessageEnvelope,
    ) -> () {
        // Messages where it matters if they are from our own scope
        if message.from_scope == self.scope {
            // Update our tools
            if let Some(mut available_tools) = Self::parse_as::<tools::ToolsAvailable>(&message) {
                self.available_tools.append(&mut available_tools.tools);
            // Handle tool call updates
            } else if let Some(tool_call_update) =
                Self::parse_as::<tools::ToolCallStatusUpdate>(&message)
            {
                self.handle_tool_call_update(tool_call_update);
            // We may make this scope agnostic but for right now only listen to status update requests from our own scope
            // We only perform the Status update if we are waiting for this tool to complete otherwise it may be an old/dropped tool or something
            } else if let Some(status_update_request) =
                Self::parse_as::<assistant::RequestStatusUpdate>(&message)
                && let Some(tool_call_id) = status_update_request.tool_call_id
            {
                if let Status::Wait {
                    reason: WaitReason::WaitingForTools { tool_calls },
                } = self.status.clone()
                {
                    if tool_calls.get(&tool_call_id).is_some() {
                        self.set_status(status_update_request.status.clone(), true);
                        if matches!(status_update_request.status, Status::Done { .. }) {
                            // TODO: Broadcast Exit?
                        }
                    }
                }
            }
        }

        // Messages where it does not matter if they are from our own scope
        if let Some(add_message) = Self::parse_as::<assistant::AddMessage>(&message)
            && add_message.agent == self.scope
        {
            match add_message.message {
                ChatMessage::System(system_chat_message) => {
                    self.pending_message.add_system_message(system_chat_message);

                    // Submit the message immediately if:
                    // 1. We are waiting for a SystemMessage from a specific scope and the message is from that scope
                    // 2. We are waiting for a SystemMessage from no specific scope
                    if let Status::Wait {
                        reason:
                            WaitReason::WaitForSystem {
                                required_scope_id, ..
                            },
                    } = &self.status
                    {
                        if required_scope_id.is_some() {
                            if required_scope_id.as_ref().unwrap() == &self.scope {
                                self.submit(false);
                            }
                        } else {
                            self.submit(false);
                        }
                    }
                }
                // Submit the message immediately if:
                // 1. We are waiting for UserInput
                // 2. We are wating for System
                // NOTE: This means a UserMessage essentially overrides the WaitForSystem state
                ChatMessage::User(user_chat_message) => {
                    self.pending_message.set_user_message(user_chat_message);
                    match self.status {
                        Status::Wait {
                            reason: WaitReason::WaitingForUserInput,
                        }
                        | Status::Wait {
                            reason: WaitReason::WaitForSystem { .. },
                        } => self.submit(false),
                        _ => (),
                    }
                }
                _ => (), // For right now we don't support adding any message besides a User or System
            }
        }
    }

    fn destructor(&mut self) -> () {
        todo!()
    }
}

use hive_actor_utils::{
    common_messages::{
        assistant::{self, Status, WaitReason},
        litellm,
        tools::{self, ToolCallStatus, ToolCallStatusUpdate},
    },
    llm_client_types::{
        self, ChatMessage, ChatRequest, ChatResponse, SystemChatMessage, Tool, UserChatMessage,
    },
};
use serde::Deserialize;

#[allow(warnings)]
mod bindings;

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantConfig {
    pub model_name: String,
    pub system_prompt: String,
    #[serde(default)]
    pub base_url: Option<String>,
}

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
    config: AssistantConfig,
    base_url: Option<String>,
}

impl Assistant {
    fn submit_with_retry(&mut self, request_id: uuid::Uuid, attempt: u32) {
        const MAX_ATTEMPTS: u32 = 3;
        const BASE_DELAY_MS: u64 = 1000; // 1 second base delay

        if attempt >= MAX_ATTEMPTS {
            tracing::error!("Max retry attempts reached for completion request");
            self.set_status(
                Status::Wait {
                    reason: WaitReason::WaitingForUserInput,
                },
                true,
            );
            return;
        }

        // Generate system prompt
        let system_prompt = self.render_system_prompt();

        // Make the completion request
        match self.make_completion_request(
            &system_prompt,
            &self.chat_history,
            Some(&self.available_tools),
        ) {
            Ok(response) => {
                // Process the response
                if let Some(choice) = response.choices.first() {
                    match &choice.message {
                        ChatMessage::Assistant(assistant_msg) => {
                            // Add response to chat history
                            self.add_chat_messages([choice.message.clone()]);

                            // Broadcast the response for other actors to handle
                            Self::broadcast(assistant::Response {
                                request_id,
                                message: assistant_msg.clone(),
                            }).unwrap();
                        }
                        _ => {
                            tracing::error!("Unexpected message type in LLM response, retrying...");
                            self.schedule_retry(request_id, attempt + 1, BASE_DELAY_MS);
                        }
                    }
                } else {
                    tracing::error!("No choices in LLM completion response, retrying...");
                    self.schedule_retry(request_id, attempt + 1, BASE_DELAY_MS);
                }
            }
            Err(e) => {
                // Check if this is a "no base URL" error - if so, go back to waiting for system or user input
                if e.contains("No LiteLLM base URL available") {
                    tracing::warn!(
                        "LiteLLM base URL not available, staying in waiting for system or user input"
                    );
                    self.set_status(
                        Status::Wait {
                            reason: WaitReason::WaitingForSystemOrUser {
                                tool_name: None,
                                tool_call_id: "no_base_url".to_string(),
                                required_scope_id: None,
                            },
                        },
                        true,
                    );
                    return;
                }

                tracing::error!(
                    "LLM completion request failed (attempt {}): {}",
                    attempt + 1,
                    e
                );
                self.schedule_retry(request_id, attempt + 1, BASE_DELAY_MS);
            }
        }
    }

    fn schedule_retry(&mut self, request_id: uuid::Uuid, attempt: u32, base_delay_ms: u64) {
        // Exponential backoff: delay = base_delay * 2^attempt
        let delay_ms = base_delay_ms * (2_u64.pow(attempt.saturating_sub(1)));

        // TODO: In a real system, we'd schedule this with a timer
        // For now, we'll just retry immediately (you could implement a timer actor)
        tracing::info!("Scheduling retry {} after {}ms", attempt + 1, delay_ms);

        // For immediate retry (would be better with actual scheduling):
        self.submit_with_retry(request_id, attempt);
    }

    fn make_completion_request(
        &self,
        system_prompt: &str,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
    ) -> Result<ChatResponse, String> {
        // Check if we have a base URL from LiteLLM
        let base_url = self.base_url.as_ref().ok_or_else(|| {
            "No LiteLLM base URL available - waiting for LiteLLM manager to start".to_string()
        })?;
        // Build the request
        let mut all_messages = vec![ChatMessage::system(system_prompt)];
        all_messages.extend_from_slice(messages);

        let request = ChatRequest {
            model: self.config.model_name.clone(),
            messages: all_messages,
            tools: tools.map(|t| t.to_vec()),
        };

        // Serialize to JSON
        let body = serde_json::to_vec(&request)
            .map_err(|e| format!("Failed to serialize request: {}", e))?;

        // Make HTTP request using our interface
        let request = bindings::hive::actor::http::Request::new(
            "POST",
            &format!("{}/v1/chat/completions", base_url),
        );

        let response = request
            .header("Content-Type", "application/json")
            .body(&body)
            .timeout(120) // 120 second timeout
            .send()
            .map_err(|e| format!("HTTP request failed: {:?}", e))?;

        // Check status
        if response.status < 200 || response.status >= 300 {
            let error_text = String::from_utf8_lossy(&response.body);
            return Err(format!("API error ({}): {}", response.status, error_text));
        }

        // Deserialize response
        serde_json::from_slice(&response.body)
            .map_err(|e| format!("Failed to deserialize response: {}", e))
    }

    fn render_system_prompt(&self) -> String {
        self.config.system_prompt.clone()
    }

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
        // Check if we have any pending messages to submit
        if !submit_if_pending_message_is_empty && !self.pending_message.has_content() {
            return;
        }

        // Add pending messages to chat history
        let new_messages = self.pending_message.to_chat_messages();
        if !new_messages.is_empty() {
            self.add_chat_messages(new_messages);
        }

        // Generate a unique request ID for this submission
        let request_id = uuid::Uuid::new_v4();

        // Set status to processing with the request ID
        self.set_status(Status::Processing { request_id }, true);

        // Start the retry process with attempt 0
        self.submit_with_retry(request_id, 0);
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
                        WaitReason::WaitingForSystemOrUser {
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
    fn new(scope: String, config_str: String) -> Self {
        let config: AssistantConfig =
            toml::from_str(&config_str).expect("Failed to parse assistant config");

        // Always start waiting for system or user input
        // Base URL will be provided either through config or broadcast
        let initial_status = Status::Wait {
            reason: WaitReason::WaitingForSystemOrUser {
                tool_name: None,
                tool_call_id: "initial".to_string(),
                required_scope_id: None,
            },
        };
        let base_url = config.base_url.clone();

        Self {
            scope,
            chat_history: vec![],
            available_tools: vec![],
            status: initial_status,
            pending_message: PendingMessage::new(),
            config,
            base_url,
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

        // Handle assistant response messages - only when we're in processing state with matching ID
        if let Some(response) = Self::parse_as::<assistant::Response>(&message) {
            if let Status::Processing { request_id } = &self.status {
                if response.request_id == *request_id {
                    // This is our response! Handle tool calls if any
                    if let Some(tool_calls) = &response.message.tool_calls {
                        // Convert tool calls to pending tool calls map
                        let mut pending_tool_calls = std::collections::HashMap::new();
                        for tool_call in tool_calls {
                            pending_tool_calls.insert(
                                tool_call.id.clone(),
                                assistant::PendingToolCall {
                                    tool_call: tool_call.clone(),
                                    result: None,
                                },
                            );
                        }

                        // Set status to waiting for tools
                        self.set_status(
                            Status::Wait {
                                reason: WaitReason::WaitingForTools {
                                    tool_calls: pending_tool_calls,
                                },
                            },
                            true,
                        );

                        // Broadcast tool calls for execution
                        for tool_call in tool_calls {
                            Self::broadcast(tools::ExecuteTool {
                                tool_call: tool_call.clone(),
                            })
                            .unwrap();
                        }
                    } else {
                        // No tool calls - we're done, wait for user input
                        self.set_status(
                            Status::Wait {
                                reason: WaitReason::WaitingForUserInput,
                            },
                            true,
                        );
                    }
                }
            }
        }

        // Handle LiteLLM base URL updates
        if let Some(base_url_update) = Self::parse_as::<litellm::BaseUrlUpdate>(&message) {
            tracing::info!(
                "Received LiteLLM base URL update: {}",
                base_url_update.base_url
            );
            self.base_url = Some(base_url_update.base_url);

            // If we were waiting for LiteLLM, transition to waiting for system or user input
            if matches!(
                self.status,
                Status::Wait {
                    reason: WaitReason::WaitingForLiteLLM
                }
            ) {
                self.set_status(
                    Status::Wait {
                        reason: WaitReason::WaitingForSystemOrUser {
                            tool_name: None,
                            tool_call_id: "base_url_received".to_string(),
                            required_scope_id: None,
                        },
                    },
                    true,
                );
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
                            WaitReason::WaitingForSystemOrUser {
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
                // 2. We are waiting for SystemOrUser
                // NOTE: This means a UserMessage essentially overrides the WaitingForSystemOrUser state
                ChatMessage::User(user_chat_message) => {
                    self.pending_message.set_user_message(user_chat_message);
                    match self.status {
                        Status::Wait {
                            reason: WaitReason::WaitingForUserInput,
                        }
                        | Status::Wait {
                            reason: WaitReason::WaitingForSystemOrUser { .. },
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

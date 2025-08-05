use bindings::hive::actor::logger;
use hive_actor_utils::{
    common_messages::{
        actors::{self, Exit},
        assistant::{
            self, ChatState, ChatStateUpdated, Request, Status, StatusUpdate,
            SystemPromptContribution, WaitReason,
        },
        litellm,
        tools::{self, ToolCallStatus, ToolCallStatusUpdate},
    },
    llm_client_types::{
        ChatMessage, ChatRequest, ChatResponse, SystemChatMessage, Tool, UserChatMessage,
    },
};
use serde::Deserialize;

#[allow(warnings)]
mod bindings;

mod system_prompt;
use system_prompt::{SystemPromptConfig, SystemPromptRenderer};

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantConfig {
    pub model_name: String,
    #[serde(default)]
    pub system_prompt: SystemPromptConfig,
    #[serde(default)]
    pub base_url: Option<String>,
    /// Whether the LLM is required to respond with tool calls
    #[serde(default)]
    pub require_tool_call: bool,
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
    available_tools: Vec<Tool>,
    status: Status,
    config: AssistantConfig,
    base_url: Option<String>,
    system_prompt_renderer: SystemPromptRenderer,
}

impl Assistant {
    fn submit_and_process(&mut self, request_id: String) {
        // Generate system prompt
        let system_prompt = self.render_system_prompt();

        // Make the completion request with automatic retry
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
                            let _ = Self::broadcast_common_message(assistant::Response {
                                request_id,
                                message: assistant_msg.clone(),
                            });
                        }
                        _ => {
                            logger::log(
                                logger::LogLevel::Error,
                                &format!("Unexpected message type in LLM response: expected Assistant message, got {:?}", choice.message)
                            );
                            self.set_status(
                                Status::Wait {
                                    reason: WaitReason::WaitingForSystemInput {
                                        required_scope: None,
                                        interruptible_by_user: true,
                                    },
                                },
                                true,
                            );
                        }
                    }
                } else {
                    logger::log(
                        logger::LogLevel::Error,
                        "LLM response contained no choices - empty response"
                    );
                    self.set_status(
                        Status::Wait {
                            reason: WaitReason::WaitingForSystemInput {
                                required_scope: None,
                                interruptible_by_user: true,
                            },
                        },
                        true,
                    );
                }
            }
            Err(e) => {
                logger::log(
                    logger::LogLevel::Error,
                    &format!("Error making completion request: {e:?}"),
                );
                self.set_status(
                    Status::Wait {
                        reason: WaitReason::WaitingForSystemInput {
                            required_scope: None,
                            interruptible_by_user: true,
                        },
                    },
                    true,
                );
            }
        }
    }

    fn make_completion_request(
        &self,
        system_prompt: &str,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
    ) -> Result<ChatResponse, String> {
        // Check if we have a base URL from LiteLLM
        let base_url = self.base_url.as_ref().ok_or_else(|| {
            "No LiteLLM base URL available! This should be impossible to reach. Please report this as a bug".to_string()
        })?;

        let system_message = match ChatMessage::system(system_prompt) {
            ChatMessage::System(msg) => msg,
            _ => unreachable!(),
        };

        let _ = Self::broadcast_common_message(Request {
            chat_state: ChatState {
                system: system_message,
                tools: self.available_tools.clone(),
                messages: self.chat_history.clone(),
            },
        });

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

        // Make HTTP request using our interface with automatic retry
        let request = bindings::hive::actor::http::Request::new(
            "POST",
            &format!("{}/v1/chat/completions", base_url),
        );

        let response = request
            .header("Content-Type", "application/json")
            .body(&body)
            .retry(3, 1000) // 3 attempts, 1 second base delay with exponential backoff
            .timeout(120) // 120 second timeout
            .send()
            .map_err(|e| {
                let error_msg = format!("HTTP request failed: {:?}", e);
                logger::log(logger::LogLevel::Error, &error_msg);
                error_msg
            })?;

        // Check status
        if response.status < 200 || response.status >= 300 {
            let error_text = String::from_utf8_lossy(&response.body);
            let error_msg = format!("LLM API error ({}): {}", response.status, error_text);
            logger::log(logger::LogLevel::Error, &error_msg);
            return Err(error_msg);
        }

        // Deserialize response
        serde_json::from_slice(&response.body).map_err(|e| {
            let error_msg = format!("Failed to deserialize response: {}", e);
            logger::log(logger::LogLevel::Error, &error_msg);
            error_msg
        })
    }

    fn render_system_prompt(&self) -> String {
        match self.system_prompt_renderer.render() {
            Ok(rendered) => rendered,
            Err(e) => {
                logger::log(
                    logger::LogLevel::Error,
                    &format!("Failed to render system prompt: {}", e),
                );
                "".to_string()
            }
        }
    }

    fn add_chat_messages(&mut self, messages: impl IntoIterator<Item = ChatMessage>) {
        self.chat_history.extend(messages);

        let system_message = match ChatMessage::system(self.render_system_prompt()) {
            ChatMessage::System(msg) => msg,
            _ => unreachable!(),
        };
        let _ = Self::broadcast_common_message(ChatStateUpdated {
            chat_state: ChatState {
                system: system_message,
                tools: self.available_tools.clone(),
                messages: self.chat_history.clone(),
            },
        });
    }

    fn set_status(&mut self, new_status: Status, broadcast_change: bool) {
        self.status = new_status;

        if broadcast_change {
            let _ = Self::broadcast_common_message(StatusUpdate {
                status: self.status.clone(),
            });
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
        let request_id = format!("req_{}", hive_actor_utils::utils::generate_id(6));

        // Set status to processing with the request ID
        self.set_status(
            Status::Processing {
                request_id: request_id.clone(),
            },
            true,
        );

        // Submit and process the request with automatic retry
        self.submit_and_process(request_id);
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
                        WaitReason::WaitingForAgentCoordination {
                            coordinating_tool_call_id,
                            coordinating_tool_name,
                            ..
                        },
                } => {
                    if coordinating_tool_call_id != &update.id {
                        return;
                    }

                    let content = result
                        .map(|tool_call_result| tool_call_result.content)
                        .unwrap_or_else(|e| format!("Error: {}", e.content));

                    self.add_chat_messages([ChatMessage::tool(
                        update.id,
                        coordinating_tool_name.clone(),
                        content,
                    )]);

                    // This checks for the following scenario:
                    // 1. We begin processing and will call the Wait tool: our state = Processing || AwaitingTools
                    // 2. We receive a user / system message we add it to the pending messages but our state remains: Processing || AwaitingTools
                    // 3. The wait too finishes
                    if self.pending_message.has_content() {
                        self.submit(false);
                    }
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

        // Start waiting for all actors to be ready before accepting any input
        // Once all actors are ready, transition based on LiteLLM availability
        let initial_status = Status::Wait {
            reason: WaitReason::WaitingForAllActorsReady,
        };
        let base_url = config.base_url.clone();

        let system_prompt_renderer =
            SystemPromptRenderer::new(config.system_prompt.clone(), scope.clone());

        Self {
            scope,
            chat_history: vec![],
            available_tools: vec![],
            status: initial_status,
            pending_message: PendingMessage::new(),
            config,
            base_url,
            system_prompt_renderer,
        }
    }

    fn handle_message(
        &mut self,
        message: bindings::exports::hive::actor::actor::MessageEnvelope,
    ) -> () {
        // Handle LiteLLM base URL updates
        if let Some(base_url_update) = Self::parse_as::<litellm::BaseUrlUpdate>(&message) {
            self.base_url = Some(base_url_update.base_url);

            // If we were waiting for LiteLLM, transition to waiting for system or user input
            if matches!(
                self.status,
                Status::Wait {
                    reason: WaitReason::WaitingForLiteLLM
                }
            ) {
                if self.pending_message.has_content() {
                    self.submit(false);
                } else {
                    self.set_status(
                        Status::Wait {
                            reason: WaitReason::WaitingForSystemInput {
                                required_scope: None,
                                interruptible_by_user: true,
                            },
                        },
                        true,
                    );
                }
            }
        }

        // Messages where it matters if they are from our own scope
        if message.from_scope == self.scope {
            if let Some(_all_actors_ready) = Self::parse_as::<actors::AllActorsReady>(&message) {
                if matches!(
                    self.status,
                    Status::Wait {
                        reason: WaitReason::WaitingForAllActorsReady
                    }
                ) {
                    // Transition based on LiteLLM availability
                    if self.base_url.is_some() {
                        if self.pending_message.has_content() {
                            self.submit(false);
                        } else {
                            self.set_status(
                                Status::Wait {
                                    reason: WaitReason::WaitingForSystemInput {
                                        required_scope: None,
                                        interruptible_by_user: true,
                                    },
                                },
                                true,
                            );
                        }
                    } else {
                        // No LiteLLM base URL, wait for it
                        self.set_status(
                            Status::Wait {
                                reason: WaitReason::WaitingForLiteLLM,
                            },
                            true,
                        );
                    }
                }
            }

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
                            let _ = Self::broadcast_common_message(Exit);
                        }
                    }
                }
            }
            // Handle assistant response messages - only when we're in processing state with matching ID
            else if let Some(response) = Self::parse_as::<assistant::Response>(&message) {
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
                                let _ = Self::broadcast_common_message(tools::ExecuteTool {
                                    tool_call: tool_call.clone(),
                                });
                            }
                        } else {
                            if self.config.require_tool_call {
                                self.pending_message.add_system_message(SystemChatMessage { content: "ERROR: It is required you respond with some kind of tool call! Review who you are and what you are doing and respond with a valid tool call".to_string() });
                                self.submit(false);
                            } else {
                                self.set_status(
                                    Status::Wait {
                                        reason: WaitReason::WaitingForSystemInput {
                                            required_scope: None,
                                            interruptible_by_user: true,
                                        },
                                    },
                                    true,
                                );
                            }
                        }
                    }
                }
            }
        }

        // We currently only accept status update requests if we are waiting on the tool that submits it
        if let Some(request_status_update) =
            Self::parse_as::<assistant::RequestStatusUpdate>(&message)
        {
            match &self.status {
                Status::Wait {
                    reason: WaitReason::WaitingForTools { tool_calls },
                } => {
                    if let Some(tool_call_id) = request_status_update.tool_call_id
                        && tool_calls.contains_key(&tool_call_id)
                    {
                        self.set_status(request_status_update.status, true);
                    }
                }
                _ => (),
            }
        }

        // Handle interrupt and force status
        if let Some(interrupt) =
            Self::parse_as::<assistant::InterruptAndForceStatus>(&message)
            && interrupt.agent == self.scope
        {
            // Check if the last message in chat history is a tool call that needs to be removed
            // This prevents errors when the manager sends a message after interruption
            if let Some(ChatMessage::Assistant(msg)) = self.chat_history.last() {
                if msg.tool_calls.is_some() {
                    // Remove the tool call message since we won't have responses for it
                    self.chat_history.pop();
                }
            }

            // Force the agent to the specified status
            self.set_status(interrupt.status, true);
        }

        // Handle add message
        if let Some(add_message) = Self::parse_as::<assistant::AddMessage>(&message)
            && add_message.agent == self.scope
        {
            match add_message.message {
                ChatMessage::System(system_chat_message) => {
                    self.pending_message.add_system_message(system_chat_message);

                    // Submit the message immediately if:
                    // 1. We are waiting for a SystemMessage from a specific scope and the message is from that scope
                    // 2. We are waiting for a SystemMessage from no specific scope
                    match &self.status {
                        Status::Wait {
                            reason: WaitReason::WaitingForSystemInput { required_scope, .. },
                        } => {
                            if let Some(required_scope) = required_scope {
                                if required_scope == &message.from_scope {
                                    self.submit(false);
                                }
                            } else {
                                self.submit(false);
                            }
                        }
                        Status::Wait {
                            reason:
                                WaitReason::WaitingForAgentCoordination {
                                    target_agent_scope, ..
                                },
                        } => {
                            if let Some(target_scope) = target_agent_scope {
                                if target_scope == &message.from_scope {
                                    self.submit(false);
                                }
                            } else {
                                self.submit(false);
                            }
                        }
                        _ => {}
                    }
                }
                // Submit the message immediately if:
                // 1. We are waiting for UserInput
                // 2. We are waiting for SystemInput with interruptible_by_user = true
                // 3. We are waiting for AgentCoordination with user_can_interrupt = true
                ChatMessage::User(user_chat_message) => {
                    self.pending_message.set_user_message(user_chat_message);
                    match self.status {
                        Status::Wait {
                            reason: WaitReason::WaitingForUserInput,
                        } => self.submit(false),
                        Status::Wait {
                            reason:
                                WaitReason::WaitingForSystemInput {
                                    interruptible_by_user: true,
                                    ..
                                },
                        } => self.submit(false),
                        Status::Wait {
                            reason:
                                WaitReason::WaitingForAgentCoordination {
                                    user_can_interrupt: true,
                                    ..
                                },
                        } => self.submit(false),
                        _ => (),
                    }
                }
                _ => (), // For right now we don't support adding any message besides a User or System
            }
        }

        // Handle system prompt contributions from any actor
        if let Some(contribution) = Self::parse_as::<SystemPromptContribution>(&message) {
            if let Err(e) = self.system_prompt_renderer.add_contribution(contribution) {
                logger::log(
                    logger::LogLevel::Error,
                    &format!("Failed to add system prompt contribution: {}", e),
                );
            }
        }
    }

    fn destructor(&mut self) -> () {}
}

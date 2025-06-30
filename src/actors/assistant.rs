use genai::{
    Client,
    chat::{ChatMessage, ChatRequest, ChatRole, ContentPart, MessageContent, Tool, ToolResponse},
};
use snafu::ResultExt;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;
use tracing::error;
use uuid::Uuid;

use crate::{
    IS_HEADLESS, SResult,
    actors::{Actor, Message, ToolCallStatus, ToolCallUpdate},
    config::ParsedModelConfig,
    scope::Scope,
    system_state::SystemState,
    template::ToolInfo,
};

use super::{
    Action, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, InterAgentMessage,
    UserContext, WaitReason,
};

/// Helper functions for formatting messages used in chat requests and tests

/// Format an agent response for successful task completion
pub fn format_agent_response_success(agent_id: &Scope, success: bool, summary: &str) -> String {
    format!(
        "<agent_response id={}>status: {}\n\n{}</agent_response>",
        agent_id,
        if success { "SUCCESS" } else { "FAILURE" },
        summary
    )
}

/// Format an agent response for failed task completion
pub fn format_agent_response_failure(agent_id: &Scope, error: &str) -> String {
    format!(
        "<agent_response id={}>status: FAILURE\n\n{}</agent_response>",
        agent_id, error
    )
}

/// Format a plan approval response
pub fn format_plan_approval_response(approved: bool, reason: Option<&str>) -> String {
    match (approved, reason) {
        (true, _) => {
            "<plan_approval_response>PLAN APPROVED BY MANAGER</plan_approval_response>".to_string()
        }
        (false, Some(reason)) => format!(
            "<plan_approval_response>PLAN REJECTED BY MANAGER: {}</plan_approval_response>",
            reason
        ),
        (false, None) => {
            "<plan_approval_response>PLAN REJECTED BY MANAGER</plan_approval_response>".to_string()
        }
    }
}

/// Format a plan approval request
pub fn format_plan_approval_request(agent_id: &str, reason: &WaitReason) -> String {
    format!(
        "<plan_approval_request agent_id={}>\n{}</plan_approval_request>",
        agent_id,
        serde_json::to_string_pretty(reason)
            .unwrap_or_else(|_| "Failed to serialize reason".to_string())
    )
}

/// Format an error message
pub fn format_error_message(error: &impl std::fmt::Display) -> String {
    format!("Error: {}", error)
}

/// Format sub agent message
pub fn format_sub_agent_message(message: &str, agent_id: &Scope) -> String {
    format!(r#"<sub_agent_message agent_id="{agent_id}">{message}</sub_agent_message>"#)
}

/// Format manager message
pub fn format_manager_message(message: &str) -> String {
    format!(r#"<manager_message>{message}</manager_message>"#)
}

/// Pending message that accumulates user input and system messages
/// to be submitted to the LLM when appropriate
#[derive(Debug, Clone, Default)]
pub struct PendingMessage {
    /// Optional user content part (only one at a time, new input replaces old)
    user_content: Option<genai::chat::ContentPart>,
    /// System message parts that accumulate from sub-agents
    system_parts: Vec<String>,
}

impl PendingMessage {
    /// Create a new empty pending message
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if the pending message has any content
    pub fn has_content(&self) -> bool {
        self.user_content.is_some() || !self.system_parts.is_empty()
    }

    /// Add or replace user content
    pub fn set_user_content(&mut self, content: genai::chat::ContentPart) {
        self.user_content = Some(content);
    }

    /// Add a system message part
    pub fn add_system_part(&mut self, message: String) {
        self.system_parts.push(message);
    }

    /// Convert to Vec<ChatMessage> for LLM submission
    /// System messages come first, then user message
    /// Returns empty vec if no content exists
    pub fn to_chat_messages(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::new();

        // Add system messages first
        for system_part in &self.system_parts {
            messages.push(ChatMessage::system(system_part.clone()));
        }

        // Add user message last if present
        if let Some(ref user_content) = self.user_content {
            messages.push(ChatMessage::user(MessageContent::Parts(vec![
                user_content.clone(),
            ])));
        }

        messages
    }

    /// Clear all content
    pub fn clear(&mut self) {
        self.user_content = None;
        self.system_parts.clear();
    }
}

/// Assistant actor that handles AI interactions
pub struct Assistant {
    tx: broadcast::Sender<ActorMessage>,
    config: ParsedModelConfig,
    client: Client,
    chat_request: ChatRequest,
    system_state: SystemState,
    available_tools: Vec<Tool>,
    cancel_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pending_message: PendingMessage,
    task_description: Option<String>,
    scope: Scope,
    parent_scope: Scope,
    spawned_agents_scope: Vec<Scope>,
    /// Whitelisted commands for the system prompt
    whitelisted_commands: Vec<String>,
    state: AgentStatus,
}

/// The assistant
/// A few limitations:
/// 1. If the assistant uses multiple tools and Wait we will lose those tools as the Wait tool puts
///    the assistant in a special state
impl Assistant {
    pub fn new(
        config: ParsedModelConfig,
        tx: broadcast::Sender<ActorMessage>,
        scope: Scope,
        parent_scope: Scope,
        required_actors: Vec<&'static str>,
        task_description: Option<String>,
        whitelisted_commands: Vec<String>,
    ) -> Self {
        let client = Client::builder()
            .with_service_target_resolver(config.service_target_resolver.clone())
            .build();

        let state = if required_actors.is_empty() {
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput,
            }
        } else {
            AgentStatus::Wait {
                reason: WaitReason::WaitingForActors {
                    pending_actors: required_actors.into_iter().map(ToOwned::to_owned).collect(),
                },
            }
        };

        let mut s = Self {
            tx,
            config,
            client,
            chat_request: ChatRequest::default(),
            system_state: SystemState::new(),
            available_tools: Vec::new(),
            cancel_handle: Arc::new(Mutex::new(None)),
            pending_message: PendingMessage::new(),
            state,
            task_description,
            scope,
            parent_scope,
            spawned_agents_scope: vec![],
            whitelisted_commands,
        };

        // If we have a task and we aren't waiting on actors just submit it here
        // We handle the case where we have a task and are waiting on actors in the handle_message
        // method
        match (&s.state, &s.task_description) {
            (AgentStatus::Wait { reason }, Some(_)) => match reason {
                WaitReason::WaitingForUserInput => {
                    // s.pending_message
                    //     .set_user_content(ContentPart::Text(s.task_description.take().unwrap()));
                    s.submit_pending_message(true);
                }
                _ => (),
            },
            _ => (),
        }

        s
    }

    /// Broadcast state change to other actors
    fn broadcast_state_change(&self) {
        let _ = self.tx.send(ActorMessage {
            scope: self.scope.clone(),
            message: Message::Agent(AgentMessage {
                agent_id: self.scope.clone(),
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdate {
                    status: self.state.clone(),
                }),
            }),
        });
    }

    /// Update state and broadcast the change
    fn set_state(&mut self, new_state: AgentStatus, broadcast_change: bool) {
        self.state = new_state;
        if broadcast_change {
            self.broadcast_state_change();
        }
    }

    fn handle_tools_available(&mut self, new_tools: Vec<Tool>) {
        // Add new tools to existing tools
        for new_tool in new_tools {
            // Remove any existing tool with the same name
            self.available_tools.retain(|t| t.name != new_tool.name);
            // Add the new tool
            self.available_tools.push(new_tool);
        }
    }

    // TODO: Add interrupt support for all tool calls
    // Useful for when the user cancels, timeouts, etc...
    fn interupt_wait_tool_call(&mut self, tool_call_id: &str, timestamp: u64, duration: Duration) {
        self.chat_request = self.chat_request.clone().append_message(ChatMessage {
            role: ChatRole::Tool,
            content: MessageContent::ToolResponses(vec![ToolResponse {
                call_id: tool_call_id.to_string(),
                content: crate::actors::tools::wait::format_wait_response_interupted(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        - timestamp,
                    duration.as_secs(),
                ),
            }]),
            options: None,
        });
        // TODO: Broadcast some kind of tool call cancelled?
    }

    fn handle_tool_call_update(&mut self, update: ToolCallUpdate) {
        if let ToolCallStatus::Finished(result) = &update.status {
            match &mut self.state {
                AgentStatus::Wait {
                    reason: WaitReason::WaitingForTools { tool_calls },
                } => {
                    let content = result.clone().unwrap_or_else(|e| format!("Error: {}", e));

                    let found = match tool_calls.get_mut(&update.call_id) {
                        Some(call) => {
                            *call = Some(content.to_string());
                            true
                        }
                        None => false,
                    };

                    if found && tool_calls.values().all(|x| x.is_some()) {
                        let pending_tool_responses = tool_calls
                            .drain()
                            .map(|(call_id, content)| ToolResponse {
                                call_id,
                                content: content.unwrap(),
                            })
                            .collect();
                        self.chat_request = self.chat_request.clone().append_message(ChatMessage {
                            role: ChatRole::Tool,
                            content: MessageContent::ToolResponses(pending_tool_responses),
                            options: None,
                        });
                        self.submit_pending_message(true);
                    }
                }
                // Don't submit pending messages while WaitingForPlanApproval
                // We need to wait for a message from our manager
                AgentStatus::Wait {
                    reason: WaitReason::WaitingForPlanApproval { tool_call_id },
                } => {
                    if tool_call_id != &update.call_id {
                        return;
                    }

                    let content = result.clone().unwrap_or_else(|e| format!("Error: {}", e));
                    self.chat_request = self.chat_request.clone().append_message(ChatMessage {
                        role: ChatRole::Tool,
                        content: MessageContent::ToolResponses(vec![ToolResponse {
                            call_id: update.call_id,
                            content,
                        }]),
                        options: None,
                    });
                }
                // Wait is a special function
                AgentStatus::Wait {
                    reason: WaitReason::WaitForDuration { tool_call_id, .. },
                } => {
                    if tool_call_id != &update.call_id {
                        return;
                    }

                    let content = result.clone().unwrap_or_else(|e| format!("Error: {}", e));
                    self.chat_request = self.chat_request.clone().append_message(ChatMessage {
                        role: ChatRole::Tool,
                        content: MessageContent::ToolResponses(vec![ToolResponse {
                            call_id: update.call_id,
                            content,
                        }]),
                        options: None,
                    });
                    self.submit_pending_message(true);
                }
                _ => (),
            }
        }
    }

    fn add_user_content(&mut self, content: ContentPart) {
        self.pending_message.set_user_content(content);
    }

    fn add_system_message_part(&mut self, message: String) {
        self.pending_message.add_system_part(message);
    }

    fn submit_pending_message(&mut self, continue_if_empty: bool) {
        if !self.pending_message.has_content() {
            if continue_if_empty {
                return self.submit_llm_request();
            } else {
                return;
            }
        }

        let messages = self.pending_message.to_chat_messages();
        self.pending_message.clear();
        for message in messages {
            self.chat_request = self.chat_request.clone().append_message(message);
        }

        self.submit_llm_request();
    }

    #[tracing::instrument(name = "llm_request", skip(self))]
    fn submit_llm_request(&mut self) {
        let processing_id = Uuid::new_v4();
        self.set_state(
            AgentStatus::Processing {
                id: processing_id.clone(),
            },
            true,
        );

        // Cancel any existing request
        if let Some(handle) = self.cancel_handle.lock().unwrap().take() {
            if !handle.is_finished() {
                handle.abort();
            }
        }

        self.maybe_rerender_system_prompt();

        // Spawn the assist task
        let tx = self.tx.clone();
        let client = self.client.clone();
        let config = self.config.clone();

        let request = self.chat_request.clone();

        // Debug log the full request
        tracing::debug!(
            "LLM Request:\n{}\n",
            serde_json::to_string_pretty(&request)
                .unwrap_or_else(|e| format!("Failed to serialize request: {}", e))
        );

        let scope = self.scope.clone();
        let handle = tokio::spawn(async move {
            if let Err(e) = do_assist(tx, client, request, config, scope, processing_id).await {
                error!("Error in assist task: {:?}", e);
            }
        });

        *self.cancel_handle.lock().unwrap() = Some(handle);
    }

    fn maybe_rerender_system_prompt(&mut self) {
        if self.system_state.is_modified() {
            // Build tool infos for system prompt
            let tool_infos: Vec<ToolInfo> = self
                .available_tools
                .iter()
                .filter_map(|tool| {
                    tool.description.as_ref().map(|desc| ToolInfo {
                        name: tool.name.clone(),
                        description: desc.clone(),
                    })
                })
                .collect();

            // Render system prompt with tools and task description
            match self.system_state.render_system_prompt_with_task(
                &self.config.system_prompt,
                &tool_infos,
                self.whitelisted_commands.clone(),
                self.task_description.clone(),
                self.scope,
            ) {
                Ok(rendered_prompt) => {
                    self.chat_request = self
                        .chat_request
                        .clone()
                        .with_system(&rendered_prompt)
                        .with_tools(self.available_tools.clone());
                    self.system_state.reset_modified();
                }
                Err(e) => {
                    error!("Failed to re-render system prompt: {}", e);
                }
            }
        }
    }
}

async fn do_assist(
    tx: broadcast::Sender<ActorMessage>,
    client: Client,
    chat_request: ChatRequest,
    config: ParsedModelConfig,
    scope: Scope,
    processing_id: Uuid,
) -> SResult<()> {
    let resp = client
        .exec_chat(&config.name, chat_request, None)
        .await
        .context(crate::GenaiSnafu)?;

    // Debug log the full response
    tracing::debug!(
        "LLM Response for agent: {scope} | content={:?}, reasoning_content={:?}, usage={:?}, model={}",
        resp.content,
        resp.reasoning_content,
        resp.usage,
        resp.model_iden.model_name
    );

    if let Some(message_content) = resp.content {
        // Note: We don't update chat_request here since it's owned by this function
        // The Assistant struct will handle updating its own chat_request when needed

        // Send response
        let _ = tx.send(ActorMessage {
            scope,
            message: Message::AssistantResponse {
                id: processing_id,
                content: message_content.clone(),
            },
        });

        // Handle tool calls if any
        if let MessageContent::ToolCalls(tool_calls) = message_content {
            for tool_call in tool_calls {
                let _ = tx.send(ActorMessage {
                    scope,
                    message: Message::AssistantToolCall(tool_call.clone()),
                });
            }
        }
    } else {
        // Something strange is happening...
        if let Some(completion_tokens) = resp.usage.completion_tokens {
            if completion_tokens > 0 {
                tracing::warn!(
                    "LLM returned no content but consumed {} completion tokens - this may be a model-specific behavior. Response details: reasoning_content={:?}, usage={:?}, model={}",
                    completion_tokens,
                    resp.reasoning_content,
                    resp.usage,
                    resp.model_iden.model_name
                );
            } else {
                error!(
                    "No message content from assistant and no tokens consumed - Response details: content={:?}, reasoning_content={:?}, usage={:?}, model={}",
                    resp.content, resp.reasoning_content, resp.usage, resp.model_iden.model_name
                );
            }
        } else {
            error!(
                "No message content from assistant - Response details: content={:?}, reasoning_content={:?}, usage={:?}, model={}",
                resp.content, resp.reasoning_content, resp.usage, resp.model_iden.model_name
            );
        }
    }

    Ok(())
}

#[async_trait::async_trait]
impl Actor for Assistant {
    const ACTOR_ID: &'static str = "assistant";

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    fn get_scope(&self) -> &Scope {
        &self.scope
    }

    fn get_scope_filters(&self) -> Vec<&Scope> {
        self.spawned_agents_scope
            .iter()
            .chain([&self.scope, &self.parent_scope])
            .collect::<Vec<&Scope>>()
    }

    async fn on_stop(&mut self) {
        for scope in self.spawned_agents_scope.drain(..) {
            let _ = self.tx.send(ActorMessage {
                scope,
                message: Message::Action(Action::Exit),
            });
        }
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        // Handle state transitions based on the message

        // If we are Done we always just return
        if matches!(self.state, AgentStatus::Done(..)) {
            return;
        }

        // message.scope == self.scope and message.agent_id == self.scope -> messages in our scope to use (typically from tools)
        // message.scope == self.parent_scope and message.agent_id == self.scope -> messages from our parent to us
        // message.scope in self.sub_agent_scopes and message.agent_id == self.scope -> messages from our children to us

        // Messages in our parent scope
        // We only care about InterAgentMessage::Message where the agent_id = our scope
        if message.scope == self.parent_scope {
            match message.message {
                Message::Agent(agent_message) if agent_message.agent_id == self.scope => {
                    match agent_message.message {
                        AgentMessageType::InterAgentMessage(inter_agent_message) => {
                            match inter_agent_message {
                                InterAgentMessage::Message { message } => {
                                    let formatted_message = format_manager_message(&message);
                                    self.add_system_message_part(formatted_message);
                                    match self.state.clone() {
                                        AgentStatus::Wait { reason } => match reason {
                                            WaitReason::WaitForDuration {
                                                tool_call_id,
                                                timestamp,
                                                duration,
                                            } => {
                                                self.interupt_wait_tool_call(
                                                    &tool_call_id,
                                                    timestamp,
                                                    duration,
                                                );
                                                self.submit_pending_message(false);
                                            }
                                            WaitReason::WaitingForUserInput
                                            | WaitReason::WaitingForPlanApproval { .. } => {
                                                self.submit_pending_message(false);
                                            }
                                            _ => (),
                                        },
                                        _ => (),
                                    }
                                }
                                _ => (),
                            }
                        }
                        _ => (),
                    }
                }
                _ => (),
            }
        // Messages from our tools, etc...
        } else if message.scope == self.scope {
            match message.message {
                Message::ActorReady { actor_id } => {
                    if let AgentStatus::Wait {
                        reason: WaitReason::WaitingForActors { pending_actors },
                    } = &mut self.state
                    {
                        *pending_actors = pending_actors
                            .drain(..)
                            .filter(|r_id| r_id != &actor_id)
                            .collect();

                        if pending_actors.is_empty() {
                            // If we have a task to execute do it!
                            // Otherwise wait for user input
                            if let Some(ref task) = self.task_description {
                                self.add_user_content(ContentPart::Text(task.clone()));
                                self.submit_pending_message(false);
                            } else {
                                self.set_state(
                                    AgentStatus::Wait {
                                        reason: WaitReason::WaitingForUserInput,
                                    },
                                    true,
                                );
                            }
                        }
                    }
                }

                Message::ToolsAvailable(tools) => self.handle_tools_available(tools),
                Message::ToolCallUpdate(update) => self.handle_tool_call_update(update),

                Message::UserContext(context) => {
                    match (context, &self.state) {
                        (
                            UserContext::UserTUIInput(text),
                            AgentStatus::Wait {
                                reason: WaitReason::WaitingForUserInput,
                            },
                        ) => {
                            self.add_user_content(ContentPart::Text(text));
                            self.submit_pending_message(false);
                        }
                        (UserContext::UserTUIInput(text), _) => {
                            self.add_user_content(ContentPart::Text(text));
                        }
                        // Other user context is handled in the tui
                        (_, _) => (),
                    }
                }

                Message::FileRead {
                    path,
                    content,
                    last_modified,
                } => {
                    self.system_state.update_file(path, content, last_modified);
                }
                Message::FileEdited {
                    path,
                    content,
                    last_modified,
                } => {
                    self.system_state.update_file(path, content, last_modified);
                }
                Message::PlanUpdated(plan) => {
                    self.system_state.update_plan(plan);
                }

                // Responses from our call to do_assist
                Message::AssistantResponse { id, content } => {
                    // State transition: Processing -> AwaitingTools or Idle/Wait based on content
                    if let AgentStatus::Processing { id: processing_id } = &self.state {
                        // This is probably an old cancelled call or something
                        if processing_id != &id {
                            return;
                        }

                        self.chat_request = self.chat_request.clone().append_message(ChatMessage {
                            role: ChatRole::Assistant,
                            content: content.clone(),
                            options: None,
                        });
                        match &content {
                            MessageContent::ToolCalls(tool_calls) => {
                                let tool_calls = tool_calls
                                    .iter()
                                    .map(|tc| (tc.call_id.clone(), None))
                                    .collect();
                                self.set_state(
                                    AgentStatus::Wait {
                                        reason: WaitReason::WaitingForTools { tool_calls },
                                    },
                                    true,
                                );
                            }
                            _ => {
                                if *crate::IS_HEADLESS.get().unwrap_or(&false) {
                                    // This is an error by the LLM it should only ever respond with
                                    // tool calls in headless mode
                                    self.pending_message.add_system_part("ERROR! You responded without calling a tool. Try again and this time ensure you call a tool! If in doubt, use the `wait` tool.".to_string());
                                    self.submit_pending_message(false);
                                } else {
                                    // If we have pending message content, submit it immediately when Idle
                                    if self.pending_message.has_content() {
                                        self.submit_pending_message(false);
                                    } else {
                                        self.set_state(
                                            AgentStatus::Wait {
                                                reason: WaitReason::WaitingForUserInput,
                                            },
                                            true,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                // Messages from our tools that update
                Message::Agent(message) => match message.message {
                    // We created a sub agent
                    AgentMessageType::AgentSpawned {
                        agent_type,
                        role,
                        task_description,
                        tool_call_id: _,
                    } => {
                        self.spawned_agents_scope.push(message.agent_id.clone());
                        let agent_info = crate::system_state::AgentTaskInfo::new(
                            message.agent_id.clone(),
                            agent_type,
                            role,
                            task_description,
                        );
                        self.system_state.add_agent(agent_info);
                    }
                    // One of our sub agents was removed
                    AgentMessageType::AgentRemoved => {
                        todo!();
                        // self.system_state.remove_agent(&message.agent_id);
                        // probably remove the scope here
                    }
                    // These are our own status updates broadcasted from tool calls
                    AgentMessageType::InterAgentMessage(inter_agent_message) => {
                        match inter_agent_message {
                            InterAgentMessage::StatusUpdateRequest { status } => {
                                self.set_state(status.clone(), true);
                                if matches!(status, AgentStatus::Done(..)) {
                                    // When we are done we shutdown
                                    let _ = self.broadcast(Message::Action(Action::Exit));
                                }
                            }
                            _ => (),
                        }
                    }
                },
                _ => {}
            }
        } else {
            // Messages from our sub agents
            match message.message {
                Message::Agent(agent_message) => match agent_message.message {
                    AgentMessageType::InterAgentMessage(inter_agent_message) => {
                        match inter_agent_message {
                            InterAgentMessage::StatusUpdate { status } => {
                                self.system_state
                                    .update_agent_status(&agent_message.agent_id, status.clone());

                                match status {
                                    AgentStatus::Done(agent_task_result_ok) => {
                                        match self.state.clone() {
                                            AgentStatus::Wait {
                                                reason:
                                                    WaitReason::WaitForDuration {
                                                        tool_call_id,
                                                        timestamp,
                                                        duration,
                                                    },
                                            } => {
                                                self.interupt_wait_tool_call(
                                                    &tool_call_id,
                                                    timestamp,
                                                    duration,
                                                );
                                                let formatted_message = match agent_task_result_ok {
                                                    Ok(response) => format_agent_response_success(
                                                        &agent_message.agent_id,
                                                        response.success,
                                                        &response.summary,
                                                    ),
                                                    Err(err) => format_agent_response_failure(
                                                        &agent_message.agent_id,
                                                        &err,
                                                    ),
                                                };
                                                self.add_system_message_part(formatted_message);
                                                self.submit_pending_message(false);
                                            }
                                            _ => {
                                                let formatted_message = match agent_task_result_ok {
                                                    Ok(response) => format_agent_response_success(
                                                        &agent_message.agent_id,
                                                        response.success,
                                                        &response.summary,
                                                    ),
                                                    Err(err) => format_agent_response_failure(
                                                        &agent_message.agent_id,
                                                        &err,
                                                    ),
                                                };
                                                self.add_system_message_part(formatted_message);
                                            }
                                        }
                                    }
                                    _ => (),
                                }
                            }
                            InterAgentMessage::Message {
                                message: sub_agent_message,
                            } if agent_message.agent_id == self.scope => {
                                let formatted_message =
                                    format_sub_agent_message(&sub_agent_message, &message.scope);
                                self.add_system_message_part(formatted_message);
                                match self.state.clone() {
                                    AgentStatus::Wait { reason } => match reason {
                                        WaitReason::WaitForDuration {
                                            tool_call_id,
                                            timestamp,
                                            duration,
                                        } => {
                                            self.interupt_wait_tool_call(
                                                &tool_call_id,
                                                timestamp,
                                                duration,
                                            );
                                            self.submit_pending_message(false);
                                        }
                                        WaitReason::WaitingForUserInput
                                        | WaitReason::WaitingForPlanApproval { .. } => {
                                            self.submit_pending_message(false);
                                        }
                                        _ => (),
                                    },
                                    _ => (),
                                }
                            }
                            _ => (),
                        }
                    }
                    _ => (),
                },
                _ => (),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::ParsedConfig;

    use super::*;

    // Test Coverage for Assistant Handle Message and State Transitions
    //
    // ## States (AgentStatus)
    //
    // 1. **Wait** with various `WaitReason` variants:
    //    - `WaitingForActors { pending_actors }` - waiting for required actors to initialize
    //    - `WaitingForUserInput` - waiting for user to provide input
    //    - `WaitingForTools { tool_calls }` - waiting for tool execution results
    //    - `WaitForDuration { tool_call_id, timestamp, duration }` - waiting for time delay
    //    - `WaitingForPlanApproval { tool_call_id }` - waiting for manager plan approval
    //
    // 2. **Processing { id }** - actively making LLM request
    //
    // ## Messages Handled by Scope
    //
    // ### 1. Messages from Parent Scope (Manager → This Agent)
    // - `Message::Agent(AgentMessage)` where `agent_id == self.scope`
    //   - `InterAgentMessage::Message { message }` - manager sending instructions
    //
    // ### 2. Messages from Own Scope (Tools/System → This Agent)
    // - `Message::ActorReady { actor_id }` - actor initialization complete
    // - `Message::ToolsAvailable(tools)` - tools broadcasting availability
    // - `Message::ToolCallUpdate(update)` - tool execution status updates
    // - `Message::UserContext(UserContext::UserTUIInput)` - user text input
    // - `Message::FileRead { path, content, last_modified }` - file read notification
    // - `Message::FileEdited { path, content, last_modified }` - file edit notification
    // - `Message::PlanUpdated(plan)` - plan state changed
    // - `Message::AssistantResponse { id, content }` - LLM response
    // - `Message::Agent(AgentMessage)`:
    //   - `AgentSpawned { agent_type, role, task_description, tool_call_id }` - sub-agent created
    //   - `AgentRemoved` - sub-agent terminated (TODO in code)
    //   - `InterAgentMessage::TaskStatusUpdate { status }` - tool updating agent status
    //
    // ### 3. Messages from Sub-Agent Scopes (Sub-Agents → This Agent)
    // - `Message::Agent(AgentMessage)`:
    //   - `InterAgentMessage::TaskStatusUpdate { status }` - sub-agent status updates
    //   - `InterAgentMessage::Message { message }` - sub-agent sending message up
    //
    // ## State Transitions
    //
    // 1. **Initial State Selection**:
    //    - With required actors → `WaitingForActors`
    //    - Without required actors → `WaitingForUserInput`
    //    - Without required actors + task → `Processing` (immediate)
    //
    // 2. **From WaitingForActors**:
    //    - All actors ready + no task → `WaitingForUserInput`
    //    - All actors ready + task → `Processing`
    //
    // 3. **From WaitingForUserInput**:
    //    - User input received → `Processing`
    //    - Manager message received → `Processing`
    //    - Sub-agent message received → `Processing`
    //
    // 4. **From Processing**:
    //    - LLM returns tool calls → `WaitingForTools`
    //    - LLM returns text + pending messages → `Processing` (re-submit)
    //    - LLM returns text + no pending → `WaitingForUserInput`
    //
    // 5. **From WaitingForTools**:
    //    - All tools complete → `Processing`
    //    - Tool requests duration wait → `WaitForDuration`
    //    - Tool requests plan approval → `WaitingForPlanApproval`
    //
    // 6. **From WaitingForPlanApproval**:
    //    - Manager/Sub-agent message received → `Processing`
    //    - (Tool will handle approval completion)
    //
    // 7. **From WaitForDuration**:
    //    - Manager/Sub-agent message received → `Processing` with interrupt tool response
    //    - Sub-agent completion (Done status) → `Processing` with interrupt tool response
    //    - (Tool will handle timeout completion)
    //
    // ## Special Behaviors
    //
    // 1. **Sub-Agent Completion Handling**:
    //    - When in `WaitForDuration`: Interrupts wait, adds response to system queue, submits LLM request
    //    - When in other states: Adds response to pending system messages (no automatic submission)
    //
    // ## Edge Cases to Test
    //
    // 1. Multiple actor ready messages
    // 2. Tool responses arriving out of order
    // 3. Old/cancelled processing responses
    // 4. Multiple pending messages accumulating
    // 5. State changes during tool execution
    // 6. Sub-agent lifecycle (spawn, status updates, removal)
    // 7. System state updates (files, plans)
    // 8. Parent vs sub-agent message filtering
    // 9. Sub-agent completion in various states

    fn create_test_assistant(
        required_actors: Vec<&'static str>,
        task_description: Option<String>,
    ) -> Assistant {
        use crate::config::Config;

        let mut config = Config::load_default(true).unwrap();
        // Set the config endpoints to complete nonsense so we don't waste tokens
        config.hive.main_manager_model.endpoint = Some("http://localhost:9000".to_string());
        config.hive.sub_manager_model.endpoint = Some("http://localhost:9000".to_string());
        config.hive.worker_model.endpoint = Some("http://localhost:9000".to_string());
        let parsed_config: ParsedConfig = config.try_into().unwrap();
        let parsed_config = parsed_config.hive.main_manager_model;

        let (tx, _) = broadcast::channel(10);
        let scope = Scope::new();
        let parent_scope = Scope::new();
        Assistant::new(
            parsed_config,
            tx,
            scope,
            parent_scope,
            required_actors,
            task_description,
            vec![],
        )
    }

    fn create_test_assistant_with_parent(
        required_actors: Vec<&'static str>,
        task_description: Option<String>,
        parent_scope: Scope,
    ) -> Assistant {
        use crate::config::Config;

        let mut config = Config::load_default(true).unwrap();
        config.hive.main_manager_model.endpoint = Some("http://localhost:9000".to_string());
        config.hive.sub_manager_model.endpoint = Some("http://localhost:9000".to_string());
        config.hive.worker_model.endpoint = Some("http://localhost:9000".to_string());
        let parsed_config: ParsedConfig = config.try_into().unwrap();
        let parsed_config = parsed_config.hive.main_manager_model;

        let (tx, _) = broadcast::channel(10);
        let scope = Scope::new();
        Assistant::new(
            parsed_config,
            tx,
            scope,
            parent_scope,
            required_actors,
            task_description,
            vec![],
        )
    }

    async fn send_message(assistant: &mut Assistant, message: Message) {
        let actor_message = ActorMessage {
            scope: assistant.scope.clone(),
            message,
        };
        assistant.handle_message(actor_message).await;
    }

    async fn send_message_with_scope(scope: &Scope, assistant: &mut Assistant, message: Message) {
        let actor_message = ActorMessage {
            scope: *scope,
            message,
        };
        assistant.handle_message(actor_message).await;
    }

    // Initial State Tests

    #[test]
    fn test_initial_state_with_required_actors() {
        let assistant = create_test_assistant(vec!["tool1", "tool2"], None);
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForActors { .. }
            }
        ));
    }

    #[test]
    fn test_initial_state_without_required_actors() {
        let assistant = create_test_assistant(vec![], None);
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));
    }

    #[tokio::test]
    async fn test_initial_state_without_required_actors_with_task() {
        let assistant = create_test_assistant(vec![], Some("Test task".to_string()));
        // Should immediately go to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));
    }

    // Actor Ready Message Tests

    #[tokio::test]
    async fn test_awaiting_actors_to_idle_transition() {
        let mut assistant = create_test_assistant(vec!["tool1", "tool2"], None);

        // Send first ActorReady
        send_message(
            &mut assistant,
            Message::ActorReady {
                actor_id: "tool1".to_string(),
            },
        )
        .await;
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForActors { .. }
            }
        ));

        // Send second ActorReady - should transition to waiting for user input
        send_message(
            &mut assistant,
            Message::ActorReady {
                actor_id: "tool2".to_string(),
            },
        )
        .await;
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));
    }

    #[tokio::test]
    async fn test_awaiting_actors_to_idle_with_task_execution() {
        let mut assistant = create_test_assistant(vec!["tool1"], Some("Test task".to_string()));

        // Verify task is stored
        assert_eq!(assistant.task_description, Some("Test task".to_string()));

        // Send ActorReady - should transition to Idle and process task
        send_message(
            &mut assistant,
            Message::ActorReady {
                actor_id: "tool1".to_string(),
            },
        )
        .await;

        // State should transition to Processing because task executes
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));

        // Chat request should contain the task as user message
        let messages = &assistant.chat_request.messages;
        assert!(!messages.is_empty());

        // Check that the last message is a user message with "Test task"
        let last_message = messages.last().unwrap();
        assert!(matches!(last_message.role, ChatRole::User));

        // Check the content
        if let MessageContent::Parts(parts) = &last_message.content {
            assert_eq!(parts.len(), 1);
            if let ContentPart::Text(text) = &parts[0] {
                assert_eq!(text, "Test task");
            } else {
                panic!("Expected text content part");
            }
        } else {
            panic!("Expected Parts content");
        }
    }

    #[tokio::test]
    async fn test_actor_ready_ignored_when_not_waiting() {
        let mut assistant = create_test_assistant(vec![], None);

        // Already in WaitingForUserInput
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));

        // Send ActorReady - should be ignored
        send_message(
            &mut assistant,
            Message::ActorReady {
                actor_id: "unexpected_tool".to_string(),
            },
        )
        .await;

        // State should remain unchanged
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));
    }

    // Tools Available Tests

    #[tokio::test]
    async fn test_tools_available_message() {
        let mut assistant = create_test_assistant(vec![], None);

        let tool1 = Tool {
            name: "test_tool1".to_string(),
            description: Some("Test tool 1".to_string()),
            schema: Some(serde_json::Value::Null),
        };
        let tool2 = Tool {
            name: "test_tool2".to_string(),
            description: Some("Test tool 2".to_string()),
            schema: Some(serde_json::Value::Null),
        };

        // Send tools available
        send_message(&mut assistant, Message::ToolsAvailable(vec![tool1.clone()])).await;
        assert_eq!(assistant.available_tools.len(), 1);
        assert_eq!(assistant.available_tools[0].name, "test_tool1");

        // Send more tools - should add to existing
        send_message(&mut assistant, Message::ToolsAvailable(vec![tool2.clone()])).await;
        assert_eq!(assistant.available_tools.len(), 2);

        // Send duplicate tool - should replace
        let tool1_updated = Tool {
            name: "test_tool1".to_string(),
            description: Some("Updated test tool 1".to_string()),
            schema: Some(serde_json::Value::Null),
        };
        send_message(&mut assistant, Message::ToolsAvailable(vec![tool1_updated])).await;
        assert_eq!(assistant.available_tools.len(), 2);
        // Find the updated tool1 by name since order may vary
        let updated_tool = assistant
            .available_tools
            .iter()
            .find(|t| t.name == "test_tool1")
            .expect("test_tool1 should exist");
        assert_eq!(
            updated_tool.description,
            Some("Updated test tool 1".to_string())
        );
    }

    // User Input Tests

    #[tokio::test]
    async fn test_user_input_when_waiting_for_input() {
        let mut assistant = create_test_assistant(vec![], None);

        // Verify in WaitingForUserInput
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));

        // Send user input
        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Hello assistant".to_string())),
        )
        .await;

        // Should transition to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));
    }

    #[tokio::test]
    async fn test_user_input_when_not_waiting_accumulates() {
        let mut assistant = create_test_assistant(vec!["tool1"], None);

        // In WaitingForActors state
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForActors { .. }
            }
        ));

        // Send user input - should accumulate but not process
        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Hello".to_string())),
        )
        .await;

        // State should remain unchanged
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForActors { .. }
            }
        ));

        // Verify pending message has content
        assert!(assistant.pending_message.has_content());
    }

    // File System State Tests

    #[tokio::test]
    async fn test_file_read_message() {
        let mut assistant = create_test_assistant(vec![], None);

        send_message(
            &mut assistant,
            Message::FileRead {
                path: std::path::PathBuf::from("/test/file.txt"),
                content: "File content".to_string(),
                last_modified: std::time::SystemTime::now(),
            },
        )
        .await;

        // System state should be modified
        assert!(assistant.system_state.is_modified());
    }

    #[tokio::test]
    async fn test_file_edited_message() {
        let mut assistant = create_test_assistant(vec![], None);

        send_message(
            &mut assistant,
            Message::FileEdited {
                path: std::path::PathBuf::from("/test/file.txt"),
                content: "Updated content".to_string(),
                last_modified: std::time::SystemTime::now(),
            },
        )
        .await;

        // System state should be modified
        assert!(assistant.system_state.is_modified());
    }

    #[tokio::test]
    async fn test_plan_updated_message() {
        use crate::actors::tools::planner::TaskPlan;

        let mut assistant = create_test_assistant(vec![], None);
        let plan = TaskPlan {
            title: "Test Plan".to_string(),
            tasks: vec![],
        };

        send_message(&mut assistant, Message::PlanUpdated(plan)).await;

        // System state should be modified
        assert!(assistant.system_state.is_modified());
    }

    // Assistant Response Tests

    #[tokio::test]
    async fn test_assistant_response_with_text() {
        let mut assistant = create_test_assistant(vec![], None);
        let processing_id = Uuid::new_v4();

        // Manually set to Processing state
        assistant.set_state(
            AgentStatus::Processing {
                id: processing_id.clone(),
            },
            true,
        );

        // Send assistant response with text
        send_message(
            &mut assistant,
            Message::AssistantResponse {
                id: processing_id,
                content: MessageContent::Text("Response text".to_string()),
            },
        )
        .await;

        // Should transition to WaitingForUserInput
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));
    }

    #[tokio::test]
    async fn test_assistant_response_with_tool_calls() {
        use genai::chat::ToolCall;

        let mut assistant = create_test_assistant(vec![], None);
        let processing_id = Uuid::new_v4();

        // Manually set to Processing state
        assistant.set_state(
            AgentStatus::Processing {
                id: processing_id.clone(),
            },
            true,
        );

        let tool_call = ToolCall {
            call_id: "call_123".to_string(),
            fn_name: "test_tool".to_string(),
            fn_arguments: serde_json::Value::Null,
        };

        // Send assistant response with tool calls
        send_message(
            &mut assistant,
            Message::AssistantResponse {
                id: processing_id,
                content: MessageContent::ToolCalls(vec![tool_call]),
            },
        )
        .await;

        // Should transition to WaitingForTools
        if let AgentStatus::Wait {
            reason: WaitReason::WaitingForTools { tool_calls },
        } = &assistant.state
        {
            assert_eq!(tool_calls.len(), 1);
            assert!(tool_calls.contains_key("call_123"));
        } else {
            panic!("Expected WaitingForTools state");
        }
    }

    #[tokio::test]
    async fn test_assistant_response_old_processing_id_ignored() {
        let mut assistant = create_test_assistant(vec![], None);
        let current_id = Uuid::new_v4();
        let old_id = Uuid::new_v4();

        // Set to Processing with current ID
        assistant.set_state(AgentStatus::Processing { id: current_id }, true);

        // Send response with old ID
        send_message(
            &mut assistant,
            Message::AssistantResponse {
                id: old_id,
                content: MessageContent::Text("Old response".to_string()),
            },
        )
        .await;

        // State should remain unchanged
        assert!(matches!(
            assistant.state,
            AgentStatus::Processing { id } if id == current_id
        ));
    }

    // Tool Call Update Tests

    #[tokio::test]
    async fn test_tool_call_update_single_tool() {
        let mut assistant = create_test_assistant(vec![], None);

        // Set up WaitingForTools state
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert("call_123".to_string(), None);
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { tool_calls },
            },
            true,
        );

        // Send tool call update
        send_message(
            &mut assistant,
            Message::ToolCallUpdate(ToolCallUpdate {
                call_id: "call_123".to_string(),
                status: ToolCallStatus::Finished(Ok("Tool result".to_string())),
            }),
        )
        .await;

        // Should transition to Processing since all tools are complete
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));
    }

    #[tokio::test]
    async fn test_tool_call_update_multiple_tools() {
        let mut assistant = create_test_assistant(vec![], None);

        // Set up WaitingForTools state with multiple tools
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert("call_1".to_string(), None);
        tool_calls.insert("call_2".to_string(), None);
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { tool_calls },
            },
            true,
        );

        // Send first tool result
        send_message(
            &mut assistant,
            Message::ToolCallUpdate(ToolCallUpdate {
                call_id: "call_1".to_string(),
                status: ToolCallStatus::Finished(Ok("Result 1".to_string())),
            }),
        )
        .await;

        // Should still be waiting
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { .. }
            }
        ));

        // Send second tool result
        send_message(
            &mut assistant,
            Message::ToolCallUpdate(ToolCallUpdate {
                call_id: "call_2".to_string(),
                status: ToolCallStatus::Finished(Ok("Result 2".to_string())),
            }),
        )
        .await;

        // Now should transition to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));
    }

    #[tokio::test]
    async fn test_tool_call_update_with_error() {
        let mut assistant = create_test_assistant(vec![], None);

        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert("call_123".to_string(), None);
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { tool_calls },
            },
            true,
        );

        // Send tool call update with error
        send_message(
            &mut assistant,
            Message::ToolCallUpdate(ToolCallUpdate {
                call_id: "call_123".to_string(),
                status: ToolCallStatus::Finished(Err("Tool error".to_string())),
            }),
        )
        .await;

        // Should still transition to Processing even with error
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));
    }

    // Agent Spawning Tests

    #[tokio::test]
    async fn test_agent_spawned_message() {
        use crate::actors::AgentType;

        let mut assistant = create_test_assistant(vec![], None);
        let spawned_agent_id = Scope::new();

        send_message(
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: spawned_agent_id,
                message: AgentMessageType::AgentSpawned {
                    agent_type: AgentType::Worker,
                    role: "test_role".to_string(),
                    task_description: "test task".to_string(),
                    tool_call_id: "call_123".to_string(),
                },
            }),
        )
        .await;

        // Should add to spawned agents
        assert_eq!(assistant.spawned_agents_scope.len(), 1);
        assert_eq!(assistant.spawned_agents_scope[0], spawned_agent_id);

        // System state should be modified
        assert!(assistant.system_state.is_modified());
    }

    // Inter-Agent Message Tests

    #[tokio::test]
    async fn test_manager_message_when_waiting() {
        let parent_scope = Scope::new();
        let mut assistant = create_test_assistant_with_parent(vec![], None, parent_scope);

        // Verify in WaitingForUserInput
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));

        // Send manager message
        let agent_scope = assistant.scope.clone();
        send_message_with_scope(
            &parent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: agent_scope,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::Message {
                    message: "Manager instruction".to_string(),
                }),
            }),
        )
        .await;

        // Should transition to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));
    }

    #[tokio::test]
    async fn test_sub_agent_message_when_waiting() {
        let mut assistant = create_test_assistant(vec![], None);
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.spawned_agents_scope.push(sub_agent_scope);

        // Verify in WaitingForUserInput
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));

        // Send sub-agent message
        let agent_scope = assistant.scope.clone();
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: agent_scope,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::Message {
                    message: "Sub-agent update".to_string(),
                }),
            }),
        )
        .await;

        // Should transition to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));
    }

    #[tokio::test]
    async fn test_sub_agent_status_update() {
        let mut assistant = create_test_assistant(vec![], None);
        let sub_agent_id = Scope::new();
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.spawned_agents_scope.push(sub_agent_scope);

        // Send sub-agent status update
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_id,
                message: AgentMessageType::InterAgentMessage(
                    InterAgentMessage::StatusUpdateRequest {
                        status: AgentStatus::Done(Ok(crate::actors::AgentTaskResultOk {
                            summary: "Task completed".to_string(),
                            success: true,
                        })),
                    },
                ),
            }),
        )
        .await;

        // System state should be modified to track sub-agent status
        assert!(assistant.system_state.is_modified());
    }

    // State Transition Tests

    #[tokio::test]
    async fn test_tool_updates_to_wait_for_duration() {
        let mut assistant = create_test_assistant(vec![], None);

        // Set up WaitingForTools state
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert("call_123".to_string(), None);
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { tool_calls },
            },
            true,
        );

        // Tool sends status update to WaitForDuration
        let agent_scope = assistant.scope.clone();
        send_message(
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: agent_scope,
                message: AgentMessageType::InterAgentMessage(
                    InterAgentMessage::StatusUpdateRequest {
                        status: AgentStatus::Wait {
                            reason: WaitReason::WaitForDuration {
                                tool_call_id: "call_123".to_string(),
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs(),
                                duration: std::time::Duration::from_secs(5),
                            },
                        },
                    },
                ),
            }),
        )
        .await;

        // Should transition to WaitForDuration
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitForDuration { .. }
            }
        ));
    }

    #[tokio::test]
    async fn test_tool_updates_to_wait_for_plan_approval() {
        let mut assistant = create_test_assistant(vec![], None);

        // Set up WaitingForTools state
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert("call_123".to_string(), None);
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { tool_calls },
            },
            true,
        );

        // Tool sends status update to WaitingForPlanApproval
        let agent_scope = assistant.scope.clone();
        send_message(
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: agent_scope,
                message: AgentMessageType::InterAgentMessage(
                    InterAgentMessage::StatusUpdateRequest {
                        status: AgentStatus::Wait {
                            reason: WaitReason::WaitingForPlanApproval {
                                tool_call_id: "call_123".to_string(),
                            },
                        },
                    },
                ),
            }),
        )
        .await;

        // Should transition to WaitingForPlanApproval
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForPlanApproval { .. }
            }
        ));
    }

    #[tokio::test]
    async fn test_manager_message_during_plan_approval_wait() {
        let parent_scope = Scope::new();
        let mut assistant = create_test_assistant_with_parent(vec![], None, parent_scope);

        // Set to WaitingForPlanApproval
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForPlanApproval {
                    tool_call_id: "call_123".to_string(),
                },
            },
            true,
        );

        // Manager message should trigger processing
        let agent_scope = assistant.scope.clone();
        send_message_with_scope(
            &parent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: agent_scope,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::Message {
                    message: "Manager approval response".to_string(),
                }),
            }),
        )
        .await;

        // Should transition to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));
    }

    #[tokio::test]
    async fn test_manager_message_during_wait_for_duration() {
        let parent_scope = Scope::new();
        let mut assistant = create_test_assistant_with_parent(vec![], None, parent_scope);

        let initial_messages_count = assistant.chat_request.messages.len();

        // Set to WaitForDuration
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let duration = std::time::Duration::from_secs(5);

        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitForDuration {
                    tool_call_id: "call_123".to_string(),
                    timestamp,
                    duration,
                },
            },
            true,
        );

        // Manager message should trigger processing and add interrupt message
        let agent_scope = assistant.scope.clone();
        send_message_with_scope(
            &parent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: agent_scope,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::Message {
                    message: "Interrupt wait duration".to_string(),
                }),
            }),
        )
        .await;

        // Should transition to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));

        // Verify that an interrupt tool and system response were added to chat_request
        assert_eq!(
            assistant.chat_request.messages.len(),
            initial_messages_count + 2
        );

        let last_message = assistant.chat_request.messages.pop().unwrap();
        assert!(matches!(last_message.role, genai::chat::ChatRole::System));

        let last_message = assistant.chat_request.messages.pop().unwrap();

        if let genai::chat::MessageContent::ToolResponses(responses) = &last_message.content {
            assert_eq!(responses.len(), 1);
            assert_eq!(responses[0].call_id, "call_123");
            // The content should contain interrupt information
            assert!(responses[0].content.contains("interrupted"));
        } else {
            panic!("Expected ToolResponses content");
        }
    }

    #[tokio::test]
    async fn test_sub_agent_message_during_wait_for_duration() {
        let mut assistant = create_test_assistant(vec![], None);
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.spawned_agents_scope.push(sub_agent_scope);

        let initial_messages_count = assistant.chat_request.messages.len();

        // Set to WaitForDuration
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let duration = std::time::Duration::from_secs(10);

        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitForDuration {
                    tool_call_id: "call_456".to_string(),
                    timestamp,
                    duration,
                },
            },
            true,
        );

        // Sub-agent message should trigger processing and add interrupt message
        let agent_scope = assistant.scope.clone();
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: agent_scope,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::Message {
                    message: "Sub-agent interrupt wait".to_string(),
                }),
            }),
        )
        .await;

        // Should transition to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));

        // Verify that an interrupt tool response was added to chat_request
        assert_eq!(
            assistant.chat_request.messages.len(),
            initial_messages_count + 2
        );

        let last_message = assistant.chat_request.messages.pop().unwrap();
        assert!(matches!(last_message.role, genai::chat::ChatRole::System));

        let last_message = assistant.chat_request.messages.pop().unwrap();

        if let genai::chat::MessageContent::ToolResponses(responses) = &last_message.content {
            assert_eq!(responses.len(), 1);
            assert_eq!(responses[0].call_id, "call_456");
            // The content should contain interrupt information
            assert!(responses[0].content.contains("interrupted"));
        } else {
            panic!("Expected ToolResponses content");
        }
    }

    #[tokio::test]
    async fn test_tool_completion_during_plan_approval_wait() {
        let mut assistant = create_test_assistant(vec![], None);

        // Set to WaitingForPlanApproval
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForPlanApproval {
                    tool_call_id: "plan_call_123".to_string(),
                },
            },
            true,
        );

        let initial_messages_count = assistant.chat_request.messages.len();

        // Send tool completion
        send_message(
            &mut assistant,
            Message::ToolCallUpdate(ToolCallUpdate {
                call_id: "plan_call_123".to_string(),
                status: ToolCallStatus::Finished(Ok("Plan submitted for approval".to_string())),
            }),
        )
        .await;

        // Should STILL be in WaitingForPlanApproval - does NOT transition to Processing
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForPlanApproval { .. }
            }
        ));

        // Tool response should be added to chat_request
        assert_eq!(
            assistant.chat_request.messages.len(),
            initial_messages_count + 1
        );

        let last_message = assistant.chat_request.messages.last().unwrap();
        assert!(matches!(last_message.role, genai::chat::ChatRole::Tool));

        if let genai::chat::MessageContent::ToolResponses(responses) = &last_message.content {
            assert_eq!(responses.len(), 1);
            assert_eq!(responses[0].call_id, "plan_call_123");
            assert_eq!(responses[0].content, "Plan submitted for approval");
        } else {
            panic!("Expected ToolResponses content");
        }
    }

    #[tokio::test]
    async fn test_tool_completion_during_wait_for_duration() {
        let mut assistant = create_test_assistant(vec![], None);

        // Set to WaitForDuration
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitForDuration {
                    tool_call_id: "wait_call_456".to_string(),
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    duration: std::time::Duration::from_secs(5),
                },
            },
            true,
        );

        let initial_messages_count = assistant.chat_request.messages.len();

        // Send tool completion
        send_message(
            &mut assistant,
            Message::ToolCallUpdate(ToolCallUpdate {
                call_id: "wait_call_456".to_string(),
                status: ToolCallStatus::Finished(Ok("Wait completed".to_string())),
            }),
        )
        .await;

        // Should transition to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));

        // Tool response should be added to chat_request
        assert_eq!(
            assistant.chat_request.messages.len(),
            initial_messages_count + 1
        );

        let last_message = assistant.chat_request.messages.last().unwrap();
        assert!(matches!(last_message.role, genai::chat::ChatRole::Tool));

        if let genai::chat::MessageContent::ToolResponses(responses) = &last_message.content {
            assert_eq!(responses.len(), 1);
            assert_eq!(responses[0].call_id, "wait_call_456");
            assert_eq!(responses[0].content, "Wait completed");
        } else {
            panic!("Expected ToolResponses content");
        }
    }

    // Pending Message Tests

    #[tokio::test]
    async fn test_pending_messages_accumulate() {
        let mut assistant = create_test_assistant(vec!["tool1"], None);

        // In WaitingForActors, messages should accumulate
        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Message 1".to_string())),
        )
        .await;

        assert!(assistant.pending_message.has_content());

        // Send another message - should replace user content
        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Message 2".to_string())),
        )
        .await;

        // Only the latest user message should be kept
        let messages = assistant.pending_message.to_chat_messages();
        assert_eq!(messages.len(), 1);
    }

    #[tokio::test]
    async fn test_assistant_response_with_pending_messages() {
        let mut assistant = create_test_assistant(vec![], None);
        let processing_id = Uuid::new_v4();

        // Add pending message
        assistant
            .pending_message
            .set_user_content(ContentPart::Text("Pending message".to_string()));

        // Set to Processing
        assistant.set_state(
            AgentStatus::Processing {
                id: processing_id.clone(),
            },
            true,
        );

        // Send assistant response
        send_message(
            &mut assistant,
            Message::AssistantResponse {
                id: processing_id,
                content: MessageContent::Text("Response".to_string()),
            },
        )
        .await;

        // Should immediately re-process due to pending message
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));
        // Pending message should be cleared
        assert!(!assistant.pending_message.has_content());
    }

    // System Prompt Re-rendering Tests

    #[tokio::test]
    async fn test_system_prompt_rerender_on_submit() {
        let mut assistant = create_test_assistant(vec![], None);

        // Add a tool to trigger system prompt update
        let tool = Tool {
            name: "test_tool".to_string(),
            description: Some("Test tool".to_string()),
            schema: Some(serde_json::Value::Null),
        };
        send_message(&mut assistant, Message::ToolsAvailable(vec![tool])).await;

        // Modify system state
        send_message(
            &mut assistant,
            Message::FileRead {
                path: std::path::PathBuf::from("/test/file.txt"),
                content: "Content".to_string(),
                last_modified: std::time::SystemTime::now(),
            },
        )
        .await;

        assert!(assistant.system_state.is_modified());

        // Trigger submit
        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Test".to_string())),
        )
        .await;

        // System state should be reset after submit
        assert!(!assistant.system_state.is_modified());
    }

    // Edge Case Tests

    #[tokio::test]
    async fn test_multiple_actor_ready_messages() {
        let mut assistant = create_test_assistant(vec!["tool1"], None);

        // Send ActorReady
        send_message(
            &mut assistant,
            Message::ActorReady {
                actor_id: "tool1".to_string(),
            },
        )
        .await;

        // Should be in WaitingForUserInput now
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));

        // Send duplicate ActorReady - should be ignored
        send_message(
            &mut assistant,
            Message::ActorReady {
                actor_id: "tool1".to_string(),
            },
        )
        .await;

        // State should remain the same
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));
    }

    #[tokio::test]
    async fn test_tool_update_for_unknown_call_id() {
        let mut assistant = create_test_assistant(vec![], None);

        // Set up WaitingForTools state
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert("call_123".to_string(), None);
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { tool_calls },
            },
            true,
        );

        // Send update for unknown call ID
        send_message(
            &mut assistant,
            Message::ToolCallUpdate(ToolCallUpdate {
                call_id: "unknown_call".to_string(),
                status: ToolCallStatus::Finished(Ok("Result".to_string())),
            }),
        )
        .await;

        // State should remain unchanged
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { .. }
            }
        ));
    }

    #[tokio::test]
    async fn test_message_filtering_by_scope() {
        let mut assistant = create_test_assistant(vec![], None);
        let unrelated_scope = Scope::new();

        // Send message from unrelated scope - should be ignored
        send_message_with_scope(
            &unrelated_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: Scope::new(),
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::Message {
                    message: "Should be ignored".to_string(),
                }),
            }),
        )
        .await;

        // State should remain unchanged
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));
    }

    #[tokio::test]
    async fn test_tool_state_transitions() {
        let mut assistant = create_test_assistant(vec![], None);

        // Set up WaitingForTools with multiple tools
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert("call_1".to_string(), None);
        tool_calls.insert("call_2".to_string(), None);
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { tool_calls },
            },
            true,
        );

        // Tool 1 requests state change to WaitForDuration
        let agent_scope = assistant.scope.clone();
        send_message(
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: agent_scope,
                message: AgentMessageType::InterAgentMessage(
                    InterAgentMessage::StatusUpdateRequest {
                        status: AgentStatus::Wait {
                            reason: WaitReason::WaitForDuration {
                                tool_call_id: "call_1".to_string(),
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs(),
                                duration: std::time::Duration::from_secs(5),
                            },
                        },
                    },
                ),
            }),
        )
        .await;

        // Should transition to WaitForDuration
        if let AgentStatus::Wait {
            reason: WaitReason::WaitForDuration { tool_call_id, .. },
        } = &assistant.state
        {
            assert_eq!(tool_call_id, "call_1");
        } else {
            panic!("Expected WaitForDuration state");
        }

        // Tool 2 can also update state - this is valid behavior
        // Tools can transition the assistant to different states as needed
        let agent_scope = assistant.scope.clone();
        send_message(
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: agent_scope,
                message: AgentMessageType::InterAgentMessage(
                    InterAgentMessage::StatusUpdateRequest {
                        status: AgentStatus::Wait {
                            reason: WaitReason::WaitingForPlanApproval {
                                tool_call_id: "call_2".to_string(),
                            },
                        },
                    },
                ),
            }),
        )
        .await;

        // State should now be WaitingForPlanApproval - tools can transition states
        if let AgentStatus::Wait {
            reason: WaitReason::WaitingForPlanApproval { tool_call_id },
        } = &assistant.state
        {
            assert_eq!(tool_call_id, "call_2");
        } else {
            panic!("Expected WaitingForPlanApproval state");
        }
    }

    // Test on_stop behavior
    #[tokio::test]
    async fn test_on_stop_sends_exit_to_sub_agents() {
        let mut assistant = create_test_assistant(vec![], None);
        let sub_agent_1 = Scope::new();
        let sub_agent_2 = Scope::new();

        // Add sub-agents
        assistant.spawned_agents_scope.push(sub_agent_1);
        assistant.spawned_agents_scope.push(sub_agent_2);

        // Create a receiver to capture messages
        let mut rx = assistant.tx.subscribe();

        // Call on_stop
        assistant.on_stop().await;

        // Should have sent exit messages to both sub-agents
        let mut exit_count = 0;
        while let Ok(msg) = rx.try_recv() {
            if matches!(msg.message, Message::Action(Action::Exit)) {
                exit_count += 1;
            }
        }
        assert_eq!(exit_count, 2);

        // Spawned agents list should be empty
        assert!(assistant.spawned_agents_scope.is_empty());
    }

    // Sub-Agent Completion Tests

    #[tokio::test]
    async fn test_sub_agent_completion_during_wait_for_duration() {
        let mut assistant = create_test_assistant(vec![], None);
        let sub_agent_id = Scope::new();
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.spawned_agents_scope.push(sub_agent_scope);

        // First spawn the sub-agent so it's in system state
        send_message(
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_id,
                message: AgentMessageType::AgentSpawned {
                    agent_type: crate::actors::AgentType::Worker,
                    role: "test_worker".to_string(),
                    task_description: "test task".to_string(),
                    tool_call_id: "spawn_123".to_string(),
                },
            }),
        )
        .await;

        // Reset system state modified flag after spawn
        assistant.system_state.reset_modified();

        let initial_messages_count = assistant.chat_request.messages.len();

        // Set to WaitForDuration
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let duration = std::time::Duration::from_secs(10);

        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitForDuration {
                    tool_call_id: "wait_call_123".to_string(),
                    timestamp,
                    duration,
                },
            },
            true,
        );

        // Send sub-agent task completion (success case)
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_id,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdate {
                    status: AgentStatus::Done(Ok(crate::actors::AgentTaskResultOk {
                        summary: "Sub-agent task completed successfully".to_string(),
                        success: true,
                    })),
                }),
            }),
        )
        .await;

        // Should transition to Processing due to interrupt
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));

        // Verify that interrupt tool response and agent response were added
        assert_eq!(
            assistant.chat_request.messages.len(),
            initial_messages_count + 2
        );

        // Check the last message is a system message with agent response
        let last_message = assistant.chat_request.messages.last().unwrap();
        assert!(matches!(last_message.role, genai::chat::ChatRole::System));
        if let MessageContent::Text(text) = &last_message.content {
            assert!(text.contains("agent_response"));
            assert!(text.contains(&sub_agent_id.to_string()));
            assert!(text.contains("SUCCESS"));
            assert!(text.contains("Sub-agent task completed successfully"));
        } else {
            panic!("Expected Text content in system message");
        }

        // Check the second to last message is a tool response (interrupt)
        let interrupt_message =
            &assistant.chat_request.messages[assistant.chat_request.messages.len() - 2];
        assert!(matches!(
            interrupt_message.role,
            genai::chat::ChatRole::Tool
        ));
        if let MessageContent::ToolResponses(responses) = &interrupt_message.content {
            assert_eq!(responses.len(), 1);
            assert_eq!(responses[0].call_id, "wait_call_123");
            assert!(responses[0].content.contains("interrupted"));
        } else {
            panic!("Expected ToolResponses content");
        }
    }

    #[tokio::test]
    async fn test_sub_agent_completion_during_wait_for_duration_failure() {
        let mut assistant = create_test_assistant(vec![], None);
        let sub_agent_id = Scope::new();
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.spawned_agents_scope.push(sub_agent_scope);

        // First spawn the sub-agent so it's in system state
        send_message(
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_id,
                message: AgentMessageType::AgentSpawned {
                    agent_type: crate::actors::AgentType::Worker,
                    role: "test_worker".to_string(),
                    task_description: "test task".to_string(),
                    tool_call_id: "spawn_456".to_string(),
                },
            }),
        )
        .await;

        // Reset system state modified flag after spawn
        assistant.system_state.reset_modified();

        let initial_messages_count = assistant.chat_request.messages.len();

        // Set to WaitForDuration
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let duration = std::time::Duration::from_secs(5);

        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitForDuration {
                    tool_call_id: "wait_call_456".to_string(),
                    timestamp,
                    duration,
                },
            },
            true,
        );

        // Send sub-agent task completion (failure case)
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_id,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdate {
                    status: AgentStatus::Done(Err("Sub-agent encountered an error".to_string())),
                }),
            }),
        )
        .await;

        // Should transition to Processing due to interrupt
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));

        // Verify messages were added
        assert_eq!(
            assistant.chat_request.messages.len(),
            initial_messages_count + 2
        );

        // Check the last message contains failure information
        let last_message = assistant.chat_request.messages.last().unwrap();
        assert!(matches!(last_message.role, genai::chat::ChatRole::System));
        if let MessageContent::Text(text) = &last_message.content {
            assert!(text.contains("agent_response"));
            assert!(text.contains(&sub_agent_id.to_string()));
            assert!(text.contains("FAILURE"));
            assert!(text.contains("Sub-agent encountered an error"));
        } else {
            panic!("Expected Text content in system message");
        }
    }

    #[tokio::test]
    async fn test_sub_agent_completion_during_waiting_for_user_input() {
        let mut assistant = create_test_assistant(vec![], None);
        let sub_agent_id = Scope::new();
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.spawned_agents_scope.push(sub_agent_scope);

        // First spawn the sub-agent so it's in system state
        send_message(
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_id,
                message: AgentMessageType::AgentSpawned {
                    agent_type: crate::actors::AgentType::Worker,
                    role: "test_worker".to_string(),
                    task_description: "background task".to_string(),
                    tool_call_id: "spawn_789".to_string(),
                },
            }),
        )
        .await;

        // Reset system state modified flag after spawn
        assistant.system_state.reset_modified();

        // Verify in WaitingForUserInput
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));

        let initial_messages_count = assistant.chat_request.messages.len();

        // Send sub-agent task completion
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_id,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdate {
                    status: AgentStatus::Done(Ok(crate::actors::AgentTaskResultOk {
                        summary: "Background task completed".to_string(),
                        success: true,
                    })),
                }),
            }),
        )
        .await;

        // Should remain in WaitingForUserInput (no automatic submission)
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));

        // System state should be modified
        assert!(assistant.system_state.is_modified());

        // No new messages should be added to chat_request
        assert_eq!(
            assistant.chat_request.messages.len(),
            initial_messages_count
        );

        // But pending message should contain the agent response
        assert!(assistant.pending_message.has_content());
        let pending_messages = assistant.pending_message.to_chat_messages();
        assert_eq!(pending_messages.len(), 1);
        assert!(matches!(pending_messages[0].role, ChatRole::System));
    }

    #[tokio::test]
    async fn test_sub_agent_completion_during_processing() {
        let mut assistant = create_test_assistant(vec![], None);
        let sub_agent_id = Scope::new();
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.spawned_agents_scope.push(sub_agent_scope);

        // Set to Processing state
        let processing_id = Uuid::new_v4();
        assistant.set_state(AgentStatus::Processing { id: processing_id }, true);

        let initial_messages_count = assistant.chat_request.messages.len();

        // Send sub-agent task completion
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_id,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdate {
                    status: AgentStatus::Done(Ok(crate::actors::AgentTaskResultOk {
                        summary: "Concurrent task finished".to_string(),
                        success: false,
                    })),
                }),
            }),
        )
        .await;

        // Should remain in Processing state
        assert!(matches!(
            assistant.state,
            AgentStatus::Processing { id } if id == processing_id
        ));

        // No new messages should be added to chat_request
        assert_eq!(
            assistant.chat_request.messages.len(),
            initial_messages_count
        );

        // But pending message should contain the agent response
        assert!(assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_sub_agent_completion_during_waiting_for_tools() {
        let mut assistant = create_test_assistant(vec![], None);
        let sub_agent_id = Scope::new();
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.spawned_agents_scope.push(sub_agent_scope);

        // Set up WaitingForTools state
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert("call_123".to_string(), None);
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { tool_calls },
            },
            true,
        );

        let initial_messages_count = assistant.chat_request.messages.len();

        // Send sub-agent task completion
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_id,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdate {
                    status: AgentStatus::Done(Ok(crate::actors::AgentTaskResultOk {
                        summary: "Helper task done".to_string(),
                        success: true,
                    })),
                }),
            }),
        )
        .await;

        // Should remain in WaitingForTools
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { .. }
            }
        ));

        // No new messages should be added to chat_request
        assert_eq!(
            assistant.chat_request.messages.len(),
            initial_messages_count
        );

        // But pending message should contain the agent response
        assert!(assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_sub_agent_completion_during_waiting_for_plan_approval() {
        let mut assistant = create_test_assistant(vec![], None);
        let sub_agent_id = Scope::new();
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.spawned_agents_scope.push(sub_agent_scope);

        // Set to WaitingForPlanApproval
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForPlanApproval {
                    tool_call_id: "plan_123".to_string(),
                },
            },
            true,
        );

        let initial_messages_count = assistant.chat_request.messages.len();

        // Send sub-agent task completion
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_id,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdate {
                    status: AgentStatus::Done(Err(
                        "Sub-agent failed while waiting for approval".to_string()
                    )),
                }),
            }),
        )
        .await;

        // Should remain in WaitingForPlanApproval
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForPlanApproval { .. }
            }
        ));

        // No new messages should be added to chat_request
        assert_eq!(
            assistant.chat_request.messages.len(),
            initial_messages_count
        );

        // But pending message should contain the agent response
        assert!(assistant.pending_message.has_content());
    }
}

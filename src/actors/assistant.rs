use genai::{
    Client,
    chat::{ChatMessage, ChatRequest, ChatRole, ContentPart, MessageContent, Tool},
};
use snafu::ResultExt;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, broadcast};
use tracing::{error, warn};

use crate::{
    SResult,
    actors::{Actor, Message, ToolCallStatus, ToolCallUpdate},
    config::ParsedModelConfig,
    scope::Scope,
    system_state::SystemState,
    template::ToolInfo,
};

use super::{
    Action, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, AgentTaskResult,
    InterAgentMessage, UserContext, WaitReason,
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

/// Context for when we're temporarily out of Wait state but need to return to it
#[derive(Debug, Clone)]
pub struct WaitContext {
    /// The original tool_call_id that put us in Wait state
    original_tool_call_id: String,
    /// Agents we're still waiting for (those without results yet)
    waiting_for_agents: HashMap<Scope, Option<AgentTaskResult>>,
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
    pending_tool_responses: Vec<genai::chat::ToolResponse>,
    pub state: AgentStatus,
    task_description: Option<String>,
    scope: Scope,
    spawned_agents_scope: Vec<Scope>,
    required_actors: Vec<&'static str>,
    /// Context for returning to Wait state after temporary Processing
    wait_context: Option<WaitContext>,
    /// Whitelisted commands for the system prompt
    whitelisted_commands: Vec<String>,
}

/// NOTE: The way this is implemented means that we do not support multiple tool calls when the
/// spawn_agents tool is used. When spawn_agents tool is used the tool_id assigned to the call is
/// assigned as the state and it overwrites the AwaitingTools status that may have a list of tools
/// to wait. Basically, we go from a potential list of tools to wait for, to one tool we are
/// waiting for. State management for which tools to wait for needs to be overhauled.
impl Assistant {
    pub fn new(
        config: ParsedModelConfig,
        tx: broadcast::Sender<ActorMessage>,
        scope: Scope,
        required_actors: Vec<&'static str>,
        task_description: Option<String>,
        whitelisted_commands: Vec<String>,
    ) -> Self {
        let client = Client::builder()
            .with_service_target_resolver(config.service_target_resolver.clone())
            .build();

        let state = if required_actors.is_empty() {
            AgentStatus::Idle
        } else {
            AgentStatus::AwaitingActors
        };

        Self {
            tx,
            config,
            client,
            chat_request: ChatRequest::default(),
            system_state: SystemState::new(),
            available_tools: Vec::new(),
            cancel_handle: Arc::new(Mutex::new(None)),
            pending_message: PendingMessage::new(),
            pending_tool_responses: Vec::new(),
            state,
            task_description,
            scope,
            required_actors,
            spawned_agents_scope: vec![],
            wait_context: None,
            whitelisted_commands,
        }
    }

    #[tracing::instrument(name = "tools_available", skip(self, new_tools), fields(tools_count = new_tools.len()))]
    async fn handle_tools_available(&mut self, new_tools: Vec<Tool>) {
        // Add new tools to existing tools
        for new_tool in new_tools {
            // Remove any existing tool with the same name
            self.available_tools.retain(|t| t.name != new_tool.name);
            // Add the new tool
            self.available_tools.push(new_tool);
        }
    }

    async fn handle_tool_call_update(&mut self, update: ToolCallUpdate) {
        if let ToolCallStatus::Finished(result) = &update.status {
            let content = result.clone().unwrap_or_else(|e| format!("Error: {}", e));

            // It is common for tools to transition our state out of AwaitingTools before broadcasting ToolCallStatus::Finished
            match &self.state {
                AgentStatus::AwaitingTools { pending_tool_calls } => {
                    if pending_tool_calls.contains(&update.call_id) {
                        let mut remaining_calls = pending_tool_calls.clone();
                        remaining_calls.retain(|id| id != &update.call_id);

                        // Create tool response and add to chat
                        let tool_response = genai::chat::ToolResponse {
                            call_id: update.call_id.clone(),
                            content,
                        };
                        self.pending_tool_responses.push(tool_response);

                        if remaining_calls.is_empty() {
                            // All tool calls complete - add tool responses to chat
                            self.chat_request =
                                self.chat_request.clone().append_message(ChatMessage {
                                    role: ChatRole::Tool,
                                    content: MessageContent::ToolResponses(std::mem::take(
                                        &mut self.pending_tool_responses,
                                    )),
                                    options: None,
                                });

                            if !self.maybe_return_to_wait() {
                                // If we have pending message content, append it before submitting
                                if self.pending_message.has_content() {
                                    let messages = self.pending_message.to_chat_messages();
                                    self.pending_message.clear();
                                    for message in messages {
                                        self.chat_request =
                                            self.chat_request.clone().append_message(message);
                                    }
                                }
                                self.submit_llm_request().await;
                            }
                        } else {
                            // Still waiting for more tools
                            self.set_state(AgentStatus::AwaitingTools {
                                pending_tool_calls: remaining_calls,
                            });
                        }
                    }
                }
                AgentStatus::Wait {
                    tool_call_id,
                    reason: _,
                } => {
                    if tool_call_id == &update.call_id {
                        let tool_response = genai::chat::ToolResponse {
                            call_id: update.call_id.clone(),
                            content,
                        };
                        self.chat_request = self.chat_request.clone().append_message(ChatMessage {
                            role: ChatRole::Tool,
                            content: MessageContent::ToolResponses(vec![tool_response]),
                            options: None,
                        });
                    }
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

    async fn submit_pending_message(&mut self) {
        if !self.pending_message.has_content() {
            return;
        }

        let messages = self.pending_message.to_chat_messages();
        self.pending_message.clear();

        for message in messages {
            self.chat_request = self.chat_request.clone().append_message(message);
        }

        self.submit_llm_request().await;
    }

    /// Check if we should return to Wait state based on wait context
    /// Returns true if we returned to Wait, false otherwise
    fn maybe_return_to_wait(&mut self) -> bool {
        if let Some(mut wait_context) = self.wait_context.take() {
            // Check if we still have agents to wait for
            let still_waiting = wait_context
                .waiting_for_agents
                .values()
                .any(|result| result.is_none());

            if still_waiting {
                // Check if we have pending user content - user input takes priority over waiting
                if self.pending_message.user_content.is_some() {
                    // User input takes priority - don't return to Wait, go to Idle/Processing instead
                    // But first add any completed agent summaries to pending messages
                    let completed_summaries = wait_context
                        .waiting_for_agents
                        .iter()
                        .filter_map(|(agent_id, result)| {
                            result.as_ref().map(|r| match r {
                                Ok(res) => format_agent_response_success(
                                    &agent_id,
                                    res.success,
                                    &res.summary,
                                ),
                                Err(err) => format_agent_response_failure(&agent_id, err),
                            })
                        })
                        .collect::<Vec<String>>();

                    if !completed_summaries.is_empty() {
                        let summary_text = completed_summaries.join("\n\n");
                        self.add_system_message_part(summary_text);
                    }

                    // Keep incomplete agents for potential future wait context
                    // For now, we lose the wait context because user input takes priority
                    return false;
                } else {
                    // No user input - return to Wait state with only system messages
                    self.wait_context = Some(wait_context.clone());
                    self.set_state(AgentStatus::Wait {
                        tool_call_id: wait_context.original_tool_call_id.clone(),
                        reason: WaitReason::WaitingForAgentResponse {
                            agent_id: crate::scope::Scope::new(), // Placeholder - managed by wait context
                        },
                    });
                    return true;
                }
            } else {
                // All agents are done - add completion summaries to pending messages
                let all_done_summaries = wait_context
                    .waiting_for_agents
                    .drain()
                    .filter_map(|(agent_id, result)| {
                        result.map(|r| match r {
                            Ok(res) => {
                                format_agent_response_success(&agent_id, res.success, &res.summary)
                            }
                            Err(err) => format_agent_response_failure(&agent_id, &err),
                        })
                    })
                    .collect::<Vec<String>>();

                if !all_done_summaries.is_empty() {
                    let summary_text = all_done_summaries.join("\n\n");
                    self.add_system_message_part(summary_text);
                }
            }
        }
        false
    }

    /// Broadcast state change to other actors
    fn broadcast_state_change(&self) {
        self.broadcast(Message::Agent(AgentMessage {
            agent_id: self.scope.clone(),
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                status: self.state.clone(),
            }),
        }));
    }

    /// Update state and broadcast the change
    fn set_state(&mut self, new_state: AgentStatus) {
        self.state = new_state;
        self.broadcast_state_change();
    }

    #[tracing::instrument(name = "llm_request", skip(self))]
    async fn submit_llm_request(&mut self) {
        if !self.pending_tool_responses.is_empty() {
            warn!("Submitting assistant request while pending_tool_responses is not empty");
        }

        self.set_state(AgentStatus::Processing);

        // Cancel any existing request
        if let Some(handle) = self.cancel_handle.lock().await.take() {
            if !handle.is_finished() {
                handle.abort();
            }
        }

        self.maybe_rerender_system_prompt().await;

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
            if let Err(e) = do_assist(tx, client, request, config, scope).await {
                error!("Error in assist task: {:?}", e);
            }
        });

        *self.cancel_handle.lock().await = Some(handle);
    }

    async fn maybe_rerender_system_prompt(&mut self) {
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
            message: Message::AssistantResponse(message_content.clone()),
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
            .chain([&self.scope])
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

        // Messages from our tools, etc...
        if message.scope == self.scope {
            match message.message {
                // Handle ActorReady messages for state transition
                Message::ActorReady { actor_id } => {
                    if let AgentStatus::AwaitingActors = &self.state {
                        self.required_actors = self
                            .required_actors
                            .drain(..)
                            .filter(|r_id| r_id != &actor_id.as_str())
                            .collect::<Vec<&'static str>>();

                        if self.required_actors.is_empty() {
                            self.set_state(AgentStatus::Idle);

                            // Check if we have a task to execute
                            if let Some(ref task) = self.task_description {
                                self.add_user_content(ContentPart::Text(task.clone()));
                                self.submit_pending_message().await;
                            }
                        }
                    }
                }

                Message::ToolsAvailable(tools) => self.handle_tools_available(tools).await,
                Message::ToolCallUpdate(update) => self.handle_tool_call_update(update).await,

                Message::UserContext(context) => {
                    match (context, &self.state) {
                        (UserContext::UserTUIInput(text), AgentStatus::Idle) => {
                            self.add_user_content(ContentPart::Text(text));
                            self.submit_pending_message().await;
                        }
                        (UserContext::UserTUIInput(text), _) => {
                            self.add_user_content(ContentPart::Text(text));
                        }
                        // Other user context is handled in the tui
                        (_, _) => (),
                        // #[cfg(feature = "audio")]
                        // (UserContext::MicrophoneTranscription(text), _) => {
                        //     self.add_user_content(ContentPart::Text(text));
                        // }
                        // #[cfg(feature = "gui")]
                        // (UserContext::ScreenshotCaptured(result), _) => {
                        //     if let Ok(base64) = result {
                        //         // Add screenshot as an image content part
                        //         let content_part = genai::chat::ContentPart::from_image_base64(
                        //             "image/png",
                        //             base64,
                        //         );
                        //         self.add_user_content(content_part);
                        //     }
                        //     // Errors are already handled by TUI
                        // }
                        // #[cfg(feature = "gui")]
                        // (UserContext::ClipboardCaptured(_result), _) => {
                        //     // Clipboard text is sent as UserTUIInput by the TUI actor when the user hits
                        //     // enter so we don't need to handle it here
                        // }
                    }
                }

                Message::Action(crate::actors::Action::Assist) => {
                    // State transition: Idle -> Processing when receiving assist action
                    if let AgentStatus::Idle = &self.state {
                        self.submit_pending_message().await;
                    } else {
                        error!("Receiving assist request in assistant when state is not Idle");
                    }
                }
                Message::Action(crate::actors::Action::Cancel) => {
                    // State transition: Processing/AwaitingTools/Wait -> Idle on cancel
                    match &self.state {
                        AgentStatus::Processing | AgentStatus::AwaitingTools { .. } => {
                            self.set_state(AgentStatus::Idle);
                            // If we have pending message content, submit it immediately
                            if self.pending_message.has_content() {
                                self.submit_pending_message().await;
                            }
                        }
                        AgentStatus::Wait { .. } => {
                            // Cancel waiting for agents - transition to Idle
                            self.set_state(AgentStatus::Idle);
                            // Clear wait context
                            self.wait_context = None;
                            // If we have pending message content, submit it immediately
                            if self.pending_message.has_content() {
                                self.submit_pending_message().await;
                            }
                        }
                        _ => {}
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

                Message::AssistantResponse(content) => {
                    // State transition: Processing -> AwaitingTools or Idle/Wait based on content
                    if let AgentStatus::Processing = &self.state {
                        self.chat_request = self.chat_request.clone().append_message(ChatMessage {
                            role: ChatRole::Assistant,
                            content: content.clone(),
                            options: None,
                        });
                        match &content {
                            MessageContent::ToolCalls(tool_calls) => {
                                let call_ids =
                                    tool_calls.iter().map(|tc| tc.call_id.clone()).collect();
                                self.set_state(AgentStatus::AwaitingTools {
                                    pending_tool_calls: call_ids,
                                });
                            }
                            _ => {
                                // Check if we should return to Wait state
                                if !self.maybe_return_to_wait() {
                                    // No wait context, go to Idle
                                    self.set_state(AgentStatus::Idle);

                                    // If we have pending message content, submit it immediately when Idle
                                    if self.pending_message.has_content() {
                                        self.submit_pending_message().await;
                                    }
                                }
                            }
                        }
                    }
                }

                Message::Agent(message) => match message.message {
                    // We created a sub agent
                    AgentMessageType::AgentSpawned {
                        agent_type,
                        role,
                        task_description,
                        tool_call_id,
                    } => {
                        self.spawned_agents_scope.push(message.agent_id.clone());
                        let agent_info = crate::system_state::AgentTaskInfo::new(
                            message.agent_id.clone(),
                            agent_type,
                            role,
                            task_description,
                        );
                        self.system_state.add_agent(agent_info);
                        if let AgentStatus::Wait {
                            tool_call_id: waiting_for_tool_call_id,
                            ..
                        } = &self.state
                        {
                            if waiting_for_tool_call_id == &tool_call_id {
                                // If we're in Wait state, we should have a wait_context
                                if let Some(ref mut wait_context) = self.wait_context {
                                    wait_context
                                        .waiting_for_agents
                                        .insert(message.agent_id, None);
                                } else {
                                    // Create wait context if it doesn't exist
                                    let mut waiting_for_agents = HashMap::new();
                                    waiting_for_agents.insert(message.agent_id, None);
                                    self.wait_context = Some(WaitContext {
                                        original_tool_call_id: tool_call_id.clone(),
                                        waiting_for_agents,
                                    });
                                }
                            }
                        }
                    }
                    // One of our sub agents was removed
                    AgentMessageType::AgentRemoved => {
                        todo!();
                        self.system_state.remove_agent(&message.agent_id);
                    }
                    // These are our own status updates broadcasted from tools, etc...
                    AgentMessageType::InterAgentMessage(inter_agent_message) => {
                        match inter_agent_message {
                            InterAgentMessage::TaskStatusUpdate { status } => {
                                // We only care about AwaitingManager and Wait states from our tools
                                match &status {
                                    super::AgentStatus::Wait {
                                        tool_call_id,
                                        reason,
                                    } => {
                                        // Handle wait state updates
                                        match &self.state {
                                            AgentStatus::Wait {
                                                tool_call_id: current_id,
                                                ..
                                            } => {
                                                if tool_call_id != current_id {
                                                    // Different tool call, update state
                                                    self.state = AgentStatus::Wait {
                                                        tool_call_id: tool_call_id.clone(),
                                                        reason: reason.clone(),
                                                    };
                                                    // Set up wait context if needed for agent spawning
                                                    if matches!(
                                                        reason,
                                                        WaitReason::WaitingForAgentResponse { .. }
                                                    ) {
                                                        self.wait_context = Some(WaitContext {
                                                            original_tool_call_id: tool_call_id
                                                                .clone(),
                                                            waiting_for_agents: HashMap::new(),
                                                        });
                                                    }
                                                }
                                                // Same tool call, preserve existing state
                                            }
                                            _ => {
                                                // Not in Wait state, transition to it
                                                self.state = AgentStatus::Wait {
                                                    tool_call_id: tool_call_id.clone(),
                                                    reason: reason.clone(),
                                                };
                                                // Set up wait context if needed for agent spawning
                                                if matches!(
                                                    reason,
                                                    WaitReason::WaitingForAgentResponse { .. }
                                                ) {
                                                    self.wait_context = Some(WaitContext {
                                                        original_tool_call_id: tool_call_id.clone(),
                                                        waiting_for_agents: HashMap::new(),
                                                    });
                                                }
                                            }
                                        }
                                    }
                                    _ => {
                                        // Ignore all other state updates
                                    }
                                }
                                self.system_state
                                    .update_agent_status(&message.agent_id, status);
                            }
                            InterAgentMessage::PlanApproved => {
                                // TODO: Maybe add plan title here?
                                // Our plan was approved by our manager - add system message and submit to LLM
                                let approval_message = "<plan_approval_response>PLAN APPROVED BY MANAGER</plan_approval_response>";
                                self.add_system_message_part(approval_message.to_string());
                                self.submit_pending_message().await;
                            }
                            InterAgentMessage::PlanRejected { reason } => {
                                // Our plan was rejected by our manager - add system message and submit to LLM
                                let rejection_message = format!(
                                    "<plan_approval_response>PLAN REJECTED BY MANAGER: {}</plan_approval_response>",
                                    reason
                                );
                                self.add_system_message_part(rejection_message);
                                self.submit_pending_message().await;
                            }
                            InterAgentMessage::ManagerMessage { message } => {
                                // Our manager sent us information - add as system message
                                let manager_message =
                                    format!("<manager_message>{}</manager_message>", message);
                                self.add_system_message_part(manager_message);

                                // If we're idle, submit immediately
                                if matches!(self.state, AgentStatus::Idle) {
                                    self.submit_pending_message().await;
                                }
                                // If we're in other states (including Wait), just queue the message
                            }
                            InterAgentMessage::SubAgentMessage {
                                message: sub_message,
                            } => {
                                // Our sub-agent sent us a message - add as system message and check if we're waiting
                                let sub_agent_message = format!(
                                    "<sub_agent_message agent_id=\"{}\">{}</sub_agent_message>",
                                    message.agent_id, sub_message
                                );
                                self.add_system_message_part(sub_agent_message);

                                // Check if we're waiting for a response from this agent
                                if let AgentStatus::Wait {
                                    reason: WaitReason::WaitingForAgentResponse { agent_id },
                                    tool_call_id,
                                } = &self.state
                                {
                                    if *agent_id == message.agent_id {
                                        // This is the response we were waiting for - complete the tool call
                                        let tool_response = genai::chat::ToolResponse {
                                            call_id: tool_call_id.clone(),
                                            content: sub_message.clone(),
                                        };
                                        self.chat_request =
                                            self.chat_request.clone().append_message(ChatMessage {
                                                role: ChatRole::Tool,
                                                content: MessageContent::ToolResponses(vec![
                                                    tool_response,
                                                ]),
                                                options: None,
                                            });
                                        // Submit the message to trigger transition to Processing
                                        self.submit_pending_message().await;
                                    }
                                }
                            }
                        }
                    }
                },
                _ => {}
            }
        } else {
            // Messages from our sub agents
            match message.message {
                Message::Agent(message) => match message.message {
                    AgentMessageType::InterAgentMessage(inter_agent_message) => {
                        match inter_agent_message {
                            InterAgentMessage::TaskStatusUpdate { status } => {
                                self.system_state
                                    .update_agent_status(&message.agent_id, status.clone());
                                match (&status, &self.state) {
                                    // ERROR: Should never receive sub-agent messages in AwaitingActors
                                    (_, AgentStatus::AwaitingActors) => {
                                        error!(
                                            "Received sub-agent status update while awaiting actors - this should not happen"
                                        );
                                    }

                                    // Sub-agent Done status handling
                                    (
                                        AgentStatus::Done(agent_task_result),
                                        AgentStatus::Wait {
                                            tool_call_id: _,
                                            reason: _,
                                        },
                                    ) => {
                                        // We're waiting for this agent - track completion in wait_context
                                        if let Some(ref mut wait_context) = self.wait_context {
                                            match wait_context
                                                .waiting_for_agents
                                                .get_mut(&message.agent_id)
                                            {
                                                Some(opt) => *opt = Some(agent_task_result.clone()),
                                                None => warn!(
                                                    "Received completion from sub-agent we aren't waiting for: {}",
                                                    message.agent_id
                                                ),
                                            }

                                            // Check if all agents we're waiting for are done
                                            if wait_context
                                                .waiting_for_agents
                                                .values()
                                                .all(|x| x.is_some())
                                            {
                                                // All agents done - compile their responses
                                                let agent_summaries = wait_context.waiting_for_agents.drain().map(|(agent_id, agent_result)| {
                                                    match agent_result.unwrap() {
                                                        Ok(res) => format!("<agent_response id={agent_id}>status: {}\n\n{}</agent_response>", 
                                                            if res.success { "SUCCESS" } else { "FAILURE" }, res.summary),
                                                        Err(err) => format!("<agent_response id={agent_id}>status: FAILURE\n\n{err}</agent_response>"),
                                                    }
                                                }).collect::<Vec<String>>();
                                                let summary_text = agent_summaries.join("\n\n");

                                                // Clear wait context since we're done waiting
                                                self.wait_context = None;

                                                // Add to pending message and submit
                                                self.add_system_message_part(summary_text);
                                                self.submit_pending_message().await;
                                            } else {
                                                // Still waiting for other agents - add this completion to pending
                                                let summary = match agent_task_result {
                                                    Ok(res) => format!(
                                                        "<agent_response id={}>status: {}\n\n{}</agent_response>",
                                                        message.agent_id,
                                                        if res.success {
                                                            "SUCCESS"
                                                        } else {
                                                            "FAILURE"
                                                        },
                                                        res.summary
                                                    ),
                                                    Err(err) => format!(
                                                        "<agent_response id={}>status: FAILURE\n\n{err}</agent_response>",
                                                        message.agent_id
                                                    ),
                                                };
                                                self.add_system_message_part(summary);
                                            }
                                        } else {
                                            error!("In Wait state but no wait_context found");
                                        }
                                    }

                                    (AgentStatus::Done(agent_task_result), AgentStatus::Idle) => {
                                        // Add completion summary to pending message and submit
                                        let summary = match agent_task_result {
                                            Ok(res) => format!(
                                                "<agent_response id={}>status: {}\n\n{}</agent_response>",
                                                message.agent_id,
                                                if res.success { "SUCCESS" } else { "FAILURE" },
                                                res.summary
                                            ),
                                            Err(err) => format!(
                                                "<agent_response id={}>status: FAILURE\n\n{err}</agent_response>",
                                                message.agent_id
                                            ),
                                        };
                                        self.add_system_message_part(summary);
                                        self.submit_pending_message().await;
                                    }

                                    (
                                        AgentStatus::Done(agent_task_result),
                                        AgentStatus::Processing
                                        | AgentStatus::AwaitingTools { .. }
                                        | AgentStatus::Wait { .. },
                                    ) => {
                                        // Update wait_context if it exists (we might be temporarily out of Wait state)
                                        if let Some(ref mut wait_context) = self.wait_context {
                                            if let Some(opt) = wait_context
                                                .waiting_for_agents
                                                .get_mut(&message.agent_id)
                                            {
                                                *opt = Some(agent_task_result.clone());
                                            }
                                        }

                                        // Add completion summary to pending message for later submission
                                        let summary = match agent_task_result {
                                            Ok(res) => format!(
                                                "<agent_response id={}>status: {}\n\n{}</agent_response>",
                                                message.agent_id,
                                                if res.success { "SUCCESS" } else { "FAILURE" },
                                                res.summary
                                            ),
                                            Err(err) => format!(
                                                "<agent_response id={}>status: FAILURE\n\n{err}</agent_response>",
                                                message.agent_id
                                            ),
                                        };
                                        self.add_system_message_part(summary);
                                    }

                                    // Sub-agent awaiting manager (plan approval) status handling
                                    (
                                        AgentStatus::Wait {
                                            reason: WaitReason::WaitingForPlanApproval,
                                            ..
                                        },
                                        AgentStatus::Wait { tool_call_id, .. },
                                    ) => {
                                        // Urgent: Plan approval needed - save wait context and transition to Processing

                                        // Wait context should already exist when in Wait state
                                        // If it doesn't, log error but continue
                                        if self.wait_context.is_none() {
                                            error!("In Wait state but no wait_context found");
                                        }

                                        let approval_request = format_plan_approval_request(
                                            &message.agent_id.to_string(),
                                            &WaitReason::WaitingForPlanApproval,
                                        );
                                        self.add_system_message_part(approval_request);
                                        self.submit_pending_message().await;
                                    }

                                    (
                                        AgentStatus::Wait {
                                            reason: WaitReason::WaitingForPlanApproval,
                                            ..
                                        },
                                        AgentStatus::Idle,
                                    ) => {
                                        // Add plan approval request to pending message and submit
                                        let approval_request = format_plan_approval_request(
                                            &message.agent_id.to_string(),
                                            &WaitReason::WaitingForPlanApproval,
                                        );
                                        self.add_system_message_part(approval_request);
                                        self.submit_pending_message().await;
                                    }

                                    (
                                        AgentStatus::Wait {
                                            reason: WaitReason::WaitingForPlanApproval,
                                            ..
                                        },
                                        AgentStatus::Processing
                                        | AgentStatus::AwaitingTools { .. }
                                        | AgentStatus::Wait { .. },
                                    ) => {
                                        // Add plan approval request to pending message for later submission
                                        let approval_request = format_plan_approval_request(
                                            &message.agent_id.to_string(),
                                            &WaitReason::WaitingForPlanApproval,
                                        );
                                        self.add_system_message_part(approval_request);
                                    }

                                    // Sub-agent waiting for manager response (from send_manager_message tool)
                                    (
                                        AgentStatus::Wait {
                                            reason: WaitReason::WaitingForManagerResponse,
                                            ..
                                        },
                                        AgentStatus::Idle,
                                    ) => {
                                        // Sub-agent needs manager response - transition to Processing immediately
                                        let manager_request = format!(
                                            "<sub_agent_message agent_id=\"{}\">[Agent is waiting for manager response]</sub_agent_message>",
                                            message.agent_id
                                        );
                                        self.add_system_message_part(manager_request);
                                        self.submit_pending_message().await;
                                    }

                                    (
                                        AgentStatus::Wait {
                                            reason: WaitReason::WaitingForManagerResponse,
                                            ..
                                        },
                                        AgentStatus::Wait { .. },
                                    ) => {
                                        // Urgent: Sub-agent needs manager response - transition to Processing
                                        let manager_request = format!(
                                            "<sub_agent_message agent_id=\"{}\">[Agent is waiting for manager response]</sub_agent_message>",
                                            message.agent_id
                                        );
                                        self.add_system_message_part(manager_request);
                                        self.submit_pending_message().await;
                                    }

                                    (
                                        AgentStatus::Wait {
                                            reason: WaitReason::WaitingForManagerResponse,
                                            ..
                                        },
                                        AgentStatus::Processing | AgentStatus::AwaitingTools { .. },
                                    ) => {
                                        // Add manager request to pending message for later submission
                                        let manager_request = format!(
                                            "<sub_agent_message agent_id=\"{}\">[Agent is waiting for manager response]</sub_agent_message>",
                                            message.agent_id
                                        );
                                        self.add_system_message_part(manager_request);
                                    }

                                    // Sub-agent InProgress status - just update system state, no action
                                    (AgentStatus::InProgress, _) => {
                                        // Update system state only - no pending message needed
                                    }

                                    // Sub-agent Waiting status - update system state only
                                    (AgentStatus::Wait { .. }, _) => {
                                        // Update system state only - no pending message needed
                                    }

                                    // Other status combinations that might occur but require no action
                                    _ => {
                                        // Update system state only - no pending message needed
                                    }
                                }
                            }
                            // Sub-agent sent us a message
                            InterAgentMessage::SubAgentMessage {
                                message: sub_message,
                            } => {
                                // A sub-agent sent us a message - add as system message
                                let sub_agent_message = format!(
                                    "<sub_agent_message agent_id=\"{}\">{}</sub_agent_message>",
                                    message.agent_id, sub_message
                                );
                                self.add_system_message_part(sub_agent_message);

                                // Check if we're waiting for a response from this agent
                                if let AgentStatus::Wait {
                                    reason: WaitReason::WaitingForAgentResponse { agent_id },
                                    tool_call_id,
                                } = &self.state
                                {
                                    if agent_id == &message.agent_id {
                                        // This is the response we were waiting for - complete the tool call
                                        let tool_response = genai::chat::ToolResponse {
                                            call_id: tool_call_id.clone(),
                                            content: sub_message.clone(),
                                        };
                                        self.chat_request =
                                            self.chat_request.clone().append_message(ChatMessage {
                                                role: ChatRole::Tool,
                                                content: MessageContent::ToolResponses(vec![
                                                    tool_response,
                                                ]),
                                                options: None,
                                            });
                                        // Continue processing will happen automatically when state transitions
                                    }
                                }

                                // If we're idle, submit immediately
                                if matches!(self.state, AgentStatus::Idle) {
                                    self.submit_pending_message().await;
                                }
                            }

                            // We don't need to handle these
                            InterAgentMessage::PlanApproved => (),
                            InterAgentMessage::PlanRejected { reason: _ } => (),
                            InterAgentMessage::ManagerMessage { .. } => (),
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
    use genai::chat::ToolCall;

    // Assistant State Transitions and Message Handling Documentation
    // ============================================================
    //
    // The Assistant actor has the following states:
    // - AwaitingActors: Initial state when required actors are not ready (brief, ~1 second)
    // - Idle: Ready to accept requests
    // - Processing: Actively making an LLM call
    // - AwaitingTools: Waiting for tool execution results
    // - Wait: Waiting with specific reasons (tool_call_id tracks the tool that caused wait):
    //   * WaitingForAgentResponse { agent_id }: Waiting for a specific agent to respond via SubAgentMessage
    //   * WaitingForManagerResponse: Waiting for our manager to respond via ManagerMessage
    //   * WaitingForPlanApproval: Waiting for our manager to approve/reject our plan
    // - Done: Task completed successfully or failed
    //
    // Wait Reasons (WaitReason enum):
    // - WaitingForAgentResponse { agent_id }: Used when manager calls send_message(wait=true)
    // - WaitingForManagerResponse: Used when agent calls send_manager_message(wait=true)
    // - WaitingForPlanApproval: Used when agent requests plan approval from manager
    //
    // Message Types:
    // - ManagerMessage: Manager -> Agent communication (from send_message tool)
    // - SubAgentMessage: Agent -> Manager communication (from send_manager_message tool)
    // - TaskStatusUpdate: State transitions broadcast by tools/agents
    //
    // Pending Message System:
    // The assistant maintains a pending message with:
    // - One optional user content part
    // - A vector of system message parts
    // This message is submitted to the LLM when conditions are met.
    //
    // STATE TRANSITIONS THAT PERFORM ACTIONS:
    //
    // 1. AwaitingActors -> Idle (on ActorReady when all actors ready):
    //    - Transition to Idle state
    //    - If task_description exists: add to pending message and submit immediately (-> Processing)
    //    - ERROR if sub-agent messages received in this state
    //
    // 2. Idle -> Processing (on UserContext, Action::Assist, or immediate messages):
    //    - UserContext: Add to pending message, submit LLM request
    //    - Action::Assist: Submit pending message to LLM (if any content exists)
    //    - ManagerMessage/SubAgentMessage: Add to pending, submit immediately
    //
    // 3. Processing -> Idle (on AssistantResponse with no tool calls):
    //    - Add response to chat history
    //    - Transition to Idle
    //    - If pending message exists: submit immediately (-> Processing)
    //
    // 4. Processing -> AwaitingTools (on AssistantResponse with tool calls):
    //    - Add response to chat history
    //    - Extract tool call IDs for tracking
    //    - Transition to AwaitingTools state
    //
    // 5. AwaitingTools -> Processing (when all tool calls complete):
    //    - Add all tool responses to chat history
    //    - Submit LLM request with tool responses
    //    - Transition to Processing
    //
    // 6. AwaitingTools -> AwaitingTools (when some tool calls complete):
    //    - Add completed tool response to pending responses
    //    - Update pending tool calls list
    //    - Remain in AwaitingTools
    //
    // 7. AwaitingTools -> Wait (when tool completes and sets wait state):
    //    - Add tool response to chat history
    //    - Set appropriate WaitReason based on tool (send_message, send_manager_message, etc.)
    //    - Track spawned agents for completion if WaitingForAgentResponse
    //    - Transition to Wait state with tool_call_id and reason
    //
    // 8. Processing/AwaitingTools -> Idle (on Action::Cancel):
    //    - Cancel current LLM request
    //    - Transition to Idle
    //    - If pending message exists: submit immediately (-> Processing)
    //
    // 9. Wait -> Processing (on sub-agent Done status when no more agents waiting):
    //     - Add agent completion summary to pending message (system part)
    //     - Submit LLM request immediately
    //     - Transition to Processing
    //
    // 10. Wait -> Wait (on sub-agent Done status when still waiting on other agents):
    //     - Add agent completion summary to pending message (system part)
    //     - Update waiting agents tracking
    //     - Remain in Wait state
    //
    // 11. Wait -> Processing (on ManagerMessage when WaitingForManagerResponse):
    //     - Add manager message to pending message (system part)
    //     - Submit LLM request immediately
    //     - Transition to Processing
    //
    // 12. Wait -> Processing (on SubAgentMessage when WaitingForAgentResponse from that agent):
    //     - Add agent message to pending message (system part)
    //     - Add tool response to chat history (completing the original tool call)
    //     - Submit LLM request immediately
    //     - Transition to Processing
    //
    // 13. Any state (on sub-agent Done status, not in Wait):
    //     - Add agent completion summary to pending message (system part)
    //     - If currently Idle: submit immediately (-> Processing)
    //     - If currently Processing/AwaitingTools: will submit after current operation completes
    //     - State remains unchanged
    //
    // 14. Any state (on sub-agent Wait { reason: WaitingForPlanApproval }):
    //     - Add plan approval request to pending message (system part) for US to review as manager
    //     - If currently Idle: submit immediately (-> Processing)
    //     - If currently Processing/AwaitingTools: will submit after current operation completes
    //     - If currently Wait: submit immediately (-> Processing) and transition out of Wait
    //     - Other states remain unchanged (WE don't transition to Wait - they are awaiting US)
    //
    // 15. Any state (on ManagerMessage when not waiting for it):
    //     - Add manager message to pending message (system part)
    //     - If currently Idle: submit immediately (-> Processing)
    //     - If currently Processing/AwaitingTools/Wait: queue in pending for later submission
    //
    // 16. Any state (on SubAgentMessage when not waiting for it):
    //     - Add agent message to pending message (system part)
    //     - If currently Idle: submit immediately (-> Processing)
    //     - If currently Processing/AwaitingTools/Wait: queue in pending for later submission
    //
    // NON-TRANSITIONING ACTIONS:
    // - UserContext while Processing/AwaitingTools/Wait: Add to pending message for later submission
    // - File/Plan updates: Update system state, mark for re-rendering
    // - Tool availability updates: Update available tools list
    // - Agent spawned: Track spawned agent scope, update system state
    // - Sub-agent InProgress status: Update system state only, no action needed
    //
    // CRITICAL CONSTRAINTS:
    // - Tool call messages MUST be followed by tool response messages
    // - When in AwaitingTools, pending messages wait until all tools complete
    // - Pending message is submitted immediately when appropriate state transitions occur
    // - Only one pending message exists at a time (new content appends to existing)
    // - Sub-agent messages in AwaitingActors state should cause an error
    // - WE are the manager when receiving sub-agent status updates
    // - Wait state with WaitingForAgentResponse completes the original tool call when receiving SubAgentMessage

    fn create_test_assistant(
        required_actors: Vec<&'static str>,
        task_description: Option<String>,
    ) -> Assistant {
        use crate::config::Config;

        let mut config = Config::default().unwrap();
        // Set the config endpoints to complete nonsense so we don't waste tokens
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
            required_actors,
            task_description,
            vec![],
        )
    }

    fn create_agent_message(agent_id: Scope, status: AgentStatus) -> ActorMessage {
        ActorMessage {
            scope: agent_id,
            message: Message::Agent(crate::actors::AgentMessage {
                agent_id,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }),
            }),
        }
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

    #[test]
    fn test_initial_state_with_required_actors() {
        let assistant = create_test_assistant(vec!["tool1", "tool2"], None);
        assert!(matches!(assistant.state, AgentStatus::AwaitingActors));
    }

    #[test]
    fn test_initial_state_without_required_actors() {
        let assistant = create_test_assistant(vec![], None);
        assert!(matches!(assistant.state, AgentStatus::Idle));
    }

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
        assert!(matches!(assistant.state, AgentStatus::AwaitingActors));

        // Send second ActorReady - should transition to Idle
        send_message(
            &mut assistant,
            Message::ActorReady {
                actor_id: "tool2".to_string(),
            },
        )
        .await;
        assert!(matches!(assistant.state, AgentStatus::Idle));
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
        assert!(matches!(assistant.state, AgentStatus::Processing));

        // Chat request should contain the task as user message
        let messages = &assistant.chat_request.messages;
        println!("{:?}", messages);
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
    async fn test_idle_to_processing_on_user_input() {
        let mut assistant = create_test_assistant(vec![], None);
        assert!(matches!(assistant.state, AgentStatus::Idle));

        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Hello".to_string())),
        )
        .await;

        assert!(matches!(assistant.state, AgentStatus::Processing));
    }

    #[tokio::test]
    async fn test_idle_to_processing_on_assist_action() {
        let mut assistant = create_test_assistant(vec![], None);
        assert!(matches!(assistant.state, AgentStatus::Idle));

        // Add some content to the pending message first
        assistant.add_user_content(ContentPart::Text("Hello".to_string()));

        send_message(
            &mut assistant,
            Message::Action(crate::actors::Action::Assist),
        )
        .await;

        assert!(matches!(assistant.state, AgentStatus::Processing));
    }

    #[tokio::test]
    async fn test_assist_action_with_no_content() {
        let mut assistant = create_test_assistant(vec![], None);
        assert!(matches!(assistant.state, AgentStatus::Idle));

        send_message(
            &mut assistant,
            Message::Action(crate::actors::Action::Assist),
        )
        .await;

        // Should remain in Idle state since there's no content to submit
        assert!(matches!(assistant.state, AgentStatus::Idle));
    }

    #[tokio::test]
    async fn test_processing_to_idle_on_text_response() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Processing;

        send_message(
            &mut assistant,
            Message::AssistantResponse(MessageContent::Text("Response".to_string())),
        )
        .await;

        assert!(matches!(assistant.state, AgentStatus::Idle));
    }

    #[tokio::test]
    async fn test_processing_to_awaiting_tools_on_tool_calls() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Processing;

        let tool_calls = vec![
            ToolCall {
                call_id: "call_123".to_string(),
                fn_name: "test_function".to_string(),
                fn_arguments: serde_json::json!({}),
            },
            ToolCall {
                call_id: "call_456".to_string(),
                fn_name: "another_function".to_string(),
                fn_arguments: serde_json::json!({}),
            },
        ];

        send_message(
            &mut assistant,
            Message::AssistantResponse(MessageContent::ToolCalls(tool_calls)),
        )
        .await;

        match &assistant.state {
            AgentStatus::AwaitingTools { pending_tool_calls } => {
                assert_eq!(pending_tool_calls.len(), 2);
                assert!(pending_tool_calls.contains(&"call_123".to_string()));
                assert!(pending_tool_calls.contains(&"call_456".to_string()));
            }
            _ => panic!("Expected AwaitingTools state"),
        }
    }

    #[tokio::test]
    async fn test_awaiting_tools_to_processing_on_all_tools_finished() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::AwaitingTools {
            pending_tool_calls: vec!["call_123".to_string()],
        };

        let update = ToolCallUpdate {
            call_id: "call_123".to_string(),
            status: ToolCallStatus::Finished(Ok("Success".to_string())),
        };

        send_message(&mut assistant, Message::ToolCallUpdate(update)).await;

        assert!(matches!(assistant.state, AgentStatus::Processing));
    }

    #[tokio::test]
    async fn test_awaiting_tools_partial_completion() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::AwaitingTools {
            pending_tool_calls: vec!["call_123".to_string(), "call_456".to_string()],
        };

        let update = ToolCallUpdate {
            call_id: "call_123".to_string(),
            status: ToolCallStatus::Finished(Ok("Success".to_string())),
        };

        send_message(&mut assistant, Message::ToolCallUpdate(update)).await;

        match &assistant.state {
            AgentStatus::AwaitingTools { pending_tool_calls } => {
                assert_eq!(pending_tool_calls.len(), 1);
                assert!(pending_tool_calls.contains(&"call_456".to_string()));
            }
            _ => panic!("Expected AwaitingTools state"),
        }
    }

    #[tokio::test]
    async fn test_cancel_from_processing_to_idle() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Processing;

        send_message(
            &mut assistant,
            Message::Action(crate::actors::Action::Cancel),
        )
        .await;

        assert!(matches!(assistant.state, AgentStatus::Idle));
    }

    #[tokio::test]
    async fn test_cancel_from_awaiting_tools_to_idle() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::AwaitingTools {
            pending_tool_calls: vec!["call_123".to_string()],
        };

        send_message(
            &mut assistant,
            Message::Action(crate::actors::Action::Cancel),
        )
        .await;

        assert!(matches!(assistant.state, AgentStatus::Idle));
    }

    #[tokio::test]
    async fn test_no_transition_user_input_while_processing() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Processing;

        assert!(!assistant.pending_message.has_content());

        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Ignored".to_string())),
        )
        .await;

        // Should remain in Processing state but should have a pending message
        assert!(assistant.pending_message.has_content());
        assert!(assistant.pending_message.user_content.is_some());
        assert!(matches!(assistant.state, AgentStatus::Processing));
    }

    #[tokio::test]
    async fn test_no_transition_user_input_while_awaiting_tools() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::AwaitingTools {
            pending_tool_calls: vec!["call_123".to_string()],
        };

        assert!(!assistant.pending_message.has_content());

        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Ignored".to_string())),
        )
        .await;

        // Should remain in AwaitingTools state but should have a pending message
        assert!(assistant.pending_message.has_content());
        assert!(assistant.pending_message.user_content.is_some());
        assert!(matches!(assistant.state, AgentStatus::AwaitingTools { .. }));
    }

    #[tokio::test]
    async fn test_no_transition_wrong_tool_call_id() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::AwaitingTools {
            pending_tool_calls: vec!["call_123".to_string()],
        };

        let update = ToolCallUpdate {
            call_id: "wrong_call_id".to_string(),
            status: ToolCallStatus::Finished(Ok("Success".to_string())),
        };

        send_message(&mut assistant, Message::ToolCallUpdate(update)).await;

        // Should remain in AwaitingTools state with same pending calls
        match &assistant.state {
            AgentStatus::AwaitingTools { pending_tool_calls } => {
                assert_eq!(pending_tool_calls.len(), 1);
                assert!(pending_tool_calls.contains(&"call_123".to_string()));
            }
            _ => panic!("Expected AwaitingTools state"),
        }
    }

    #[tokio::test]
    async fn test_no_transition_actor_ready_when_idle() {
        let mut assistant = create_test_assistant(vec![], None);
        assert!(matches!(assistant.state, AgentStatus::Idle));

        send_message(
            &mut assistant,
            Message::ActorReady {
                actor_id: "unexpected".to_string(),
            },
        )
        .await;

        // Should remain in Idle state
        assert!(matches!(assistant.state, AgentStatus::Idle));
    }

    // Tests for the new PendingMessage system
    #[test]
    fn test_pending_message_empty() {
        let msg = PendingMessage::new();
        assert!(!msg.has_content());
        assert!(msg.to_chat_messages().is_empty());
    }

    #[test]
    fn test_pending_message_user_content_only() {
        let mut msg = PendingMessage::new();
        msg.set_user_content(ContentPart::Text("Hello".to_string()));

        assert!(msg.has_content());
        let chat_messages = msg.to_chat_messages();
        assert_eq!(chat_messages.len(), 1);
        assert!(matches!(chat_messages[0].role, ChatRole::User));
    }

    #[test]
    fn test_pending_message_system_parts_only() {
        let mut msg = PendingMessage::new();
        msg.add_system_part("System message 1".to_string());
        msg.add_system_part("System message 2".to_string());

        assert!(msg.has_content());
        let chat_messages = msg.to_chat_messages();
        assert_eq!(chat_messages.len(), 2);
        assert!(matches!(chat_messages[0].role, ChatRole::System));
        assert!(matches!(chat_messages[1].role, ChatRole::System));
    }

    #[test]
    fn test_pending_message_mixed_content() {
        let mut msg = PendingMessage::new();
        msg.add_system_part("System message 1".to_string());
        msg.add_system_part("System message 2".to_string());
        msg.set_user_content(ContentPart::Text("User message".to_string()));

        assert!(msg.has_content());
        let chat_messages = msg.to_chat_messages();
        assert_eq!(chat_messages.len(), 3);

        // System messages come first
        assert!(matches!(chat_messages[0].role, ChatRole::System));
        assert!(matches!(chat_messages[1].role, ChatRole::System));
        // User message comes last
        assert!(matches!(chat_messages[2].role, ChatRole::User));
    }

    #[test]
    fn test_pending_message_user_content_replacement() {
        let mut msg = PendingMessage::new();
        msg.set_user_content(ContentPart::Text("First user message".to_string()));
        msg.set_user_content(ContentPart::Text("Second user message".to_string()));

        let chat_messages = msg.to_chat_messages();
        assert_eq!(chat_messages.len(), 1);

        // Should contain the second message only
        if let MessageContent::Parts(ref parts) = chat_messages[0].content {
            if let ContentPart::Text(ref text) = parts[0] {
                assert_eq!(text, "Second user message");
            } else {
                panic!("Expected text content part");
            }
        } else {
            panic!("Expected Parts content");
        }
    }

    #[test]
    fn test_pending_message_clear() {
        let mut msg = PendingMessage::new();
        msg.set_user_content(ContentPart::Text("User message".to_string()));
        msg.add_system_part("System message".to_string());

        assert!(msg.has_content());

        msg.clear();
        assert!(!msg.has_content());
        assert!(msg.to_chat_messages().is_empty());
    }

    #[tokio::test]
    async fn test_sub_agent_done_in_idle_state() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Idle;

        let agent_id = Scope::new();
        let task_result = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Task completed".to_string(),
        });

        let agent_message = create_agent_message(agent_id, AgentStatus::Done(task_result));
        assistant.handle_message(agent_message).await;

        // Should transition to Processing after receiving sub-agent completion
        assert!(matches!(assistant.state, AgentStatus::Processing));

        // The pending message should have been cleared after submission to LLM
        assert!(!assistant.pending_message.has_content());

        // Verify that the chat request now contains the agent response
        let messages = &assistant.chat_request.messages;
        assert!(!messages.is_empty());
        // Should have system message containing agent response
        let last_message = messages.last().unwrap();
        assert!(matches!(last_message.role, ChatRole::System));
    }

    #[tokio::test]
    async fn test_sub_agent_done_while_processing() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Processing;

        let agent_id = Scope::new();
        let task_result = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Task completed".to_string(),
        });

        let agent_message = create_agent_message(agent_id, AgentStatus::Done(task_result));
        assistant.handle_message(agent_message).await;

        // Should remain in Processing state
        assert!(matches!(assistant.state, AgentStatus::Processing));

        // Should have added system message to pending message for later submission
        assert!(!assistant.pending_message.system_parts.is_empty());
    }

    #[tokio::test]
    async fn test_sub_agent_awaiting_manager_in_idle() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Idle;

        let agent_id = Scope::new();
        let agent_message = create_agent_message(
            agent_id,
            AgentStatus::Wait {
                tool_call_id: "info_123".to_string(),
                reason: WaitReason::WaitingForManagerResponse,
            },
        );
        assistant.handle_message(agent_message).await;

        // Should transition to Processing for plan approval
        assert!(matches!(assistant.state, AgentStatus::Processing));

        // The pending message should have been cleared after submission to LLM
        assert!(!assistant.pending_message.has_content());

        // Verify that the chat request now contains the plan approval request
        let messages = &assistant.chat_request.messages;
        assert!(!messages.is_empty());
        let last_message = messages.last().unwrap();
        assert!(matches!(last_message.role, ChatRole::System));
    }

    #[tokio::test]
    async fn test_sub_agent_awaiting_manager_while_waiting() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Wait {
            tool_call_id: "tool123".to_string(),
            reason: WaitReason::WaitingForManagerResponse,
        };

        let agent_id = Scope::new();
        let agent_message = create_agent_message(
            agent_id,
            AgentStatus::Wait {
                tool_call_id: "info_123".to_string(),
                reason: WaitReason::WaitingForManagerResponse,
            },
        );
        assistant.handle_message(agent_message).await;

        // Should transition out of Wait to Processing for urgent plan approval
        assert!(matches!(assistant.state, AgentStatus::Processing));

        // The pending message should have been cleared after submission to LLM
        assert!(!assistant.pending_message.has_content());

        // Verify that the chat request now contains the plan approval request
        let messages = &assistant.chat_request.messages;
        assert!(!messages.is_empty());
        let last_message = messages.last().unwrap();
        assert!(matches!(last_message.role, ChatRole::System));
    }

    #[tokio::test]
    async fn test_sub_agent_done_while_waiting_single_agent() {
        let mut assistant = create_test_assistant(vec![], None);
        let agent_id = Scope::new();

        // Set up Wait state with one agent
        assistant.state = AgentStatus::Wait {
            tool_call_id: "tool123".to_string(),
            reason: WaitReason::WaitingForManagerResponse,
        };
        let mut waiting_for_agents = HashMap::new();
        waiting_for_agents.insert(agent_id, None);
        assistant.wait_context = Some(WaitContext {
            original_tool_call_id: "tool123".to_string(),
            waiting_for_agents,
        });

        let task_result = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Task completed".to_string(),
        });

        let agent_message = create_agent_message(agent_id, AgentStatus::Done(task_result));
        assistant.handle_message(agent_message).await;

        // Should transition to Processing since all agents are done
        assert!(matches!(assistant.state, AgentStatus::Processing));

        // wait_context should be cleared (we transitioned out of Wait)
        assert!(assistant.wait_context.is_none());

        // The pending message should have been cleared after submission to LLM
        assert!(!assistant.pending_message.has_content());

        // Verify that the chat request now contains the agent response
        let messages = &assistant.chat_request.messages;
        assert!(!messages.is_empty());
        let last_message = messages.last().unwrap();
        assert!(matches!(last_message.role, ChatRole::System));
    }

    #[tokio::test]
    async fn test_sub_agent_done_while_waiting_multiple_agents() {
        let mut assistant = create_test_assistant(vec![], None);
        let agent1_id = Scope::new();
        let agent2_id = Scope::new();

        // Set up Wait state with two agents
        assistant.state = AgentStatus::Wait {
            tool_call_id: "tool123".to_string(),
            reason: WaitReason::WaitingForManagerResponse,
        };
        let mut waiting_for_agents = HashMap::new();
        waiting_for_agents.insert(agent1_id, None);
        waiting_for_agents.insert(agent2_id, None);
        assistant.wait_context = Some(WaitContext {
            original_tool_call_id: "tool123".to_string(),
            waiting_for_agents,
        });

        let task_result = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Agent 1 completed".to_string(),
        });

        // First agent completes
        let agent_message = create_agent_message(agent1_id, AgentStatus::Done(task_result));
        assistant.handle_message(agent_message).await;

        // Should remain in Wait state since agent2 is still pending
        assert!(matches!(assistant.state, AgentStatus::Wait { .. }));

        // Check wait_context for agent tracking
        assert!(assistant.wait_context.is_some());
        let wait_context = assistant.wait_context.as_ref().unwrap();
        assert_eq!(wait_context.waiting_for_agents.len(), 2);
        assert!(
            wait_context
                .waiting_for_agents
                .get(&agent1_id)
                .unwrap()
                .is_some()
        );
        assert!(
            wait_context
                .waiting_for_agents
                .get(&agent2_id)
                .unwrap()
                .is_none()
        );

        // Should have added partial response to pending message
        assert!(!assistant.pending_message.system_parts.is_empty());
    }

    #[tokio::test]
    async fn test_sub_agent_in_progress_no_action() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Idle;

        let agent_id = Scope::new();
        let agent_message = create_agent_message(agent_id, AgentStatus::InProgress);
        assistant.handle_message(agent_message).await;

        // Should remain in same state
        assert!(matches!(assistant.state, AgentStatus::Idle));

        // Should not add anything to pending message
        assert!(!assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_sub_agent_waiting_no_action() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Idle;

        let agent_id = Scope::new();
        let agent_message = create_agent_message(
            agent_id,
            AgentStatus::Wait {
                tool_call_id: "tool123".to_string(),
                reason: WaitReason::WaitingForAgentResponse { agent_id },
            },
        );
        assistant.handle_message(agent_message).await;

        // Should remain in same state
        assert!(matches!(assistant.state, AgentStatus::Idle));

        // Should not add anything to pending message
        assert!(!assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_user_input_accumulates_in_pending_message() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Processing;

        // Add user input while processing
        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Hello".to_string())),
        )
        .await;

        // Should remain in Processing state
        assert!(matches!(assistant.state, AgentStatus::Processing));

        // Should accumulate in pending message
        assert!(assistant.pending_message.has_content());
        assert!(assistant.pending_message.user_content.is_some());
    }

    #[tokio::test]
    async fn test_wait_state_recovery_after_plan_approval() {
        let mut assistant = create_test_assistant(vec![], None);
        let agent1_id = Scope::new();
        let agent2_id = Scope::new();

        // Set up Wait state with two agents
        assistant.state = AgentStatus::Wait {
            tool_call_id: "spawn_agents_123".to_string(),
            reason: WaitReason::WaitingForAgentResponse {
                agent_id: crate::scope::Scope::new(),
            },
        };
        let mut waiting_for_agents = HashMap::new();
        waiting_for_agents.insert(agent1_id, None);
        waiting_for_agents.insert(agent2_id, None);
        assistant.wait_context = Some(WaitContext {
            original_tool_call_id: "spawn_agents_123".to_string(),
            waiting_for_agents,
        });

        // Agent1 requests plan approval
        let agent_message = create_agent_message(
            agent1_id,
            AgentStatus::Wait {
                tool_call_id: "info_123".to_string(),
                reason: WaitReason::WaitingForManagerResponse,
            },
        );
        assistant.handle_message(agent_message).await;

        // Should transition to Processing for plan approval
        assert!(matches!(assistant.state, AgentStatus::Processing));

        // Wait context should be saved
        assert!(assistant.wait_context.is_some());
        let wait_context = assistant.wait_context.as_ref().unwrap();
        assert_eq!(wait_context.original_tool_call_id, "spawn_agents_123");
        assert_eq!(wait_context.waiting_for_agents.len(), 2);

        // Simulate LLM response (plan approval)
        send_message(
            &mut assistant,
            Message::AssistantResponse(MessageContent::Text("Plan approved".to_string())),
        )
        .await;

        // Should return to Wait state since agent2 is still pending
        assert!(matches!(assistant.state, AgentStatus::Wait { .. }));
        if let AgentStatus::Wait { tool_call_id, .. } = &assistant.state {
            assert_eq!(tool_call_id, "spawn_agents_123");
        }

        // Wait context should be restored since we're back in Wait state
        assert!(assistant.wait_context.is_some());
    }

    #[tokio::test]
    async fn test_agent_completes_while_approving_plan() {
        let mut assistant = create_test_assistant(vec![], None);
        let agent1_id = Scope::new();
        let agent2_id = Scope::new();

        // Set up Wait state with two agents
        assistant.state = AgentStatus::Wait {
            tool_call_id: "spawn_agents_123".to_string(),
            reason: WaitReason::WaitingForAgentResponse {
                agent_id: crate::scope::Scope::new(),
            },
        };
        let mut waiting_for_agents = HashMap::new();
        waiting_for_agents.insert(agent1_id, None);
        waiting_for_agents.insert(agent2_id, None);
        assistant.wait_context = Some(WaitContext {
            original_tool_call_id: "spawn_agents_123".to_string(),
            waiting_for_agents,
        });

        // Agent1 requests plan approval
        let agent_message = create_agent_message(
            agent1_id,
            AgentStatus::Wait {
                tool_call_id: "info_123".to_string(),
                reason: WaitReason::WaitingForManagerResponse,
            },
        );
        assistant.handle_message(agent_message).await;

        // Should transition to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing));

        // While processing, agent2 completes
        let task_result = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Agent2 completed".to_string(),
        });
        let agent_message = create_agent_message(agent2_id, AgentStatus::Done(task_result));
        assistant.handle_message(agent_message).await;

        // Should remain in Processing
        assert!(matches!(assistant.state, AgentStatus::Processing));

        // But wait_context should be updated
        assert!(assistant.wait_context.is_some());
        let wait_context = assistant.wait_context.as_ref().unwrap();
        assert!(
            wait_context
                .waiting_for_agents
                .get(&agent2_id)
                .unwrap()
                .is_some()
        );

        // Pending message should have agent2's completion
        assert!(!assistant.pending_message.system_parts.is_empty());
    }

    #[tokio::test]
    async fn test_all_agents_complete_while_processing_returns_to_idle() {
        let mut assistant = create_test_assistant(vec![], None);
        let agent1_id = Scope::new();
        let agent2_id = Scope::new();

        // Set up Wait state with two agents
        assistant.state = AgentStatus::Wait {
            tool_call_id: "spawn_agents_123".to_string(),
            reason: WaitReason::WaitingForAgentResponse {
                agent_id: crate::scope::Scope::new(),
            },
        };
        let mut waiting_for_agents = HashMap::new();
        waiting_for_agents.insert(agent1_id, None);
        waiting_for_agents.insert(agent2_id, None);
        assistant.wait_context = Some(WaitContext {
            original_tool_call_id: "spawn_agents_123".to_string(),
            waiting_for_agents,
        });

        // Agent1 requests plan approval
        let agent_message = create_agent_message(
            agent1_id,
            AgentStatus::Wait {
                tool_call_id: "info_123".to_string(),
                reason: WaitReason::WaitingForManagerResponse,
            },
        );
        assistant.handle_message(agent_message).await;

        // Both agents complete while processing
        let task_result1 = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Agent1 completed".to_string(),
        });
        let agent_message1 = create_agent_message(agent1_id, AgentStatus::Done(task_result1));
        assistant.handle_message(agent_message1).await;

        let task_result2 = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Agent2 completed".to_string(),
        });
        let agent_message2 = create_agent_message(agent2_id, AgentStatus::Done(task_result2));
        assistant.handle_message(agent_message2).await;

        // Still in Processing
        assert!(matches!(assistant.state, AgentStatus::Processing));

        // Simulate LLM response
        send_message(
            &mut assistant,
            Message::AssistantResponse(MessageContent::Text("Done".to_string())),
        )
        .await;

        // Should go to Processing since we're submitting the agent completions
        assert!(matches!(assistant.state, AgentStatus::Processing));

        // Pending message should be cleared since it was submitted
        assert!(!assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_wait_context_preserved_across_multiple_transitions() {
        let mut assistant = create_test_assistant(vec![], None);
        let agent1_id = Scope::new();
        let agent2_id = Scope::new();
        let agent3_id = Scope::new();

        // Set up Wait state with three agents
        assistant.state = AgentStatus::Wait {
            tool_call_id: "spawn_agents_123".to_string(),
            reason: WaitReason::WaitingForAgentResponse {
                agent_id: crate::scope::Scope::new(),
            },
        };
        let mut waiting_for_agents = HashMap::new();
        waiting_for_agents.insert(agent1_id, None);
        waiting_for_agents.insert(agent2_id, None);
        waiting_for_agents.insert(agent3_id, None);
        assistant.wait_context = Some(WaitContext {
            original_tool_call_id: "spawn_agents_123".to_string(),
            waiting_for_agents,
        });

        // Agent1 requests plan approval
        let agent_message = create_agent_message(
            agent1_id,
            AgentStatus::Wait {
                tool_call_id: "info_123".to_string(),
                reason: WaitReason::WaitingForManagerResponse,
            },
        );
        assistant.handle_message(agent_message).await;

        // Agent2 completes while processing agent1's plan
        let task_result = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Agent2 completed".to_string(),
        });
        let agent_message = create_agent_message(agent2_id, AgentStatus::Done(task_result));
        assistant.handle_message(agent_message).await;

        // Finish plan approval
        send_message(
            &mut assistant,
            Message::AssistantResponse(MessageContent::Text("Plan approved".to_string())),
        )
        .await;

        // Should return to Wait state (agent1 and agent3 still pending)
        assert!(matches!(assistant.state, AgentStatus::Wait { .. }));

        // Agent3 requests plan approval
        let agent_message = create_agent_message(
            agent3_id,
            AgentStatus::Wait {
                tool_call_id: "info_123".to_string(),
                reason: WaitReason::WaitingForManagerResponse,
            },
        );
        assistant.handle_message(agent_message).await;

        // Finish plan approval
        send_message(
            &mut assistant,
            Message::AssistantResponse(MessageContent::Text(
                "Plan approved for agent3".to_string(),
            )),
        )
        .await;

        // Should return to Wait state since agent1 and agent3 are still not done
        assert!(matches!(assistant.state, AgentStatus::Wait { .. }));

        // Agent1 completes
        let task_result = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Agent1 completed".to_string(),
        });
        let agent_message = create_agent_message(agent1_id, AgentStatus::Done(task_result));
        assistant.handle_message(agent_message).await;

        // Still waiting for agent3
        assert!(matches!(assistant.state, AgentStatus::Wait { .. }));

        // Agent3 completes
        let task_result = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Agent3 completed".to_string(),
        });
        let agent_message = create_agent_message(agent3_id, AgentStatus::Done(task_result));
        assistant.handle_message(agent_message).await;

        // Should transition to Processing to report all completions
        assert!(matches!(assistant.state, AgentStatus::Processing));
    }

    #[tokio::test]
    async fn test_manager_message_while_idle() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Idle;

        let agent_message = Message::Agent(AgentMessage {
            agent_id: assistant.scope, // Message to self
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::ManagerMessage {
                message: "Here is the information you requested".to_string(),
            }),
        });

        assistant
            .handle_message(ActorMessage {
                scope: assistant.scope,
                message: agent_message,
            })
            .await;

        // Should transition to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing));
        // Pending message should be cleared after submission
        assert!(!assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_manager_message_while_processing() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Processing;

        let agent_message = Message::Agent(AgentMessage {
            agent_id: assistant.scope,
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::ManagerMessage {
                message: "Additional information".to_string(),
            }),
        });

        assistant
            .handle_message(ActorMessage {
                scope: assistant.scope,
                message: agent_message,
            })
            .await;

        // Should remain in Processing
        assert!(matches!(assistant.state, AgentStatus::Processing));
        // Message should be queued in pending
        assert!(assistant.pending_message.has_content());
        assert!(assistant.pending_message.system_parts.iter().any(|part| {
            part.contains("<manager_message>Additional information</manager_message>")
        }));
    }

    #[tokio::test]
    async fn test_manager_message_while_awaiting_tools() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::AwaitingTools {
            pending_tool_calls: vec!["tool123".to_string()],
        };

        let agent_message = Message::Agent(AgentMessage {
            agent_id: assistant.scope,
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::ManagerMessage {
                message: "Information while waiting for tools".to_string(),
            }),
        });

        assistant
            .handle_message(ActorMessage {
                scope: assistant.scope,
                message: agent_message,
            })
            .await;

        // Should remain in AwaitingTools
        assert!(matches!(assistant.state, AgentStatus::AwaitingTools { .. }));
        // Message should be queued
        assert!(assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_manager_message_while_wait() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Wait {
            tool_call_id: "wait123".to_string(),
            reason: WaitReason::WaitingForManagerResponse,
        };

        let agent_message = Message::Agent(AgentMessage {
            agent_id: assistant.scope,
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::ManagerMessage {
                message: "Information while waiting".to_string(),
            }),
        });

        assistant
            .handle_message(ActorMessage {
                scope: assistant.scope,
                message: agent_message,
            })
            .await;

        // Should remain in Wait
        assert!(matches!(assistant.state, AgentStatus::Wait { .. }));
        // Message should be queued
        assert!(assistant.pending_message.has_content());
        assert!(assistant.pending_message.system_parts.iter().any(|part| {
            part.contains("<manager_message>Information while waiting</manager_message>")
        }));
    }

    #[tokio::test]
    async fn test_wait_manager_response_to_processing() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Wait {
            tool_call_id: "wait123".to_string(),
            reason: WaitReason::WaitingForManagerResponse,
        };

        assert!(!assistant.pending_message.has_content());

        let agent_message = Message::Agent(AgentMessage {
            agent_id: assistant.scope,
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::ManagerMessage {
                message: "Response from manager".to_string(),
            }),
        });

        assistant
            .handle_message(ActorMessage {
                scope: assistant.scope,
                message: agent_message,
            })
            .await;

        // Should remain in Wait state and queue the message
        assert!(matches!(assistant.state, AgentStatus::Wait { .. }));
        // Message should be queued
        assert!(assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_wait_agent_response_to_processing_and_complete_tool_call() {
        let mut assistant = create_test_assistant(vec![], None);
        let agent_id = Scope::new();

        assistant.state = AgentStatus::Wait {
            tool_call_id: "send_msg_123".to_string(),
            reason: WaitReason::WaitingForAgentResponse { agent_id },
        };

        // Agent sends back a response
        let agent_message = Message::Agent(AgentMessage {
            agent_id,
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::SubAgentMessage {
                message: "Response from agent".to_string(),
            }),
        });

        assistant
            .handle_message(ActorMessage {
                scope: assistant.scope,
                message: agent_message,
            })
            .await;

        // Should transition to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing));
        // Should have completed tool call and submitted message (so pending is clear)
        assert!(!assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_sub_agent_message_while_processing() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::Processing;
        let agent_id = Scope::new();

        let agent_message = Message::Agent(AgentMessage {
            agent_id,
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::SubAgentMessage {
                message: "Message while processing".to_string(),
            }),
        });

        assistant
            .handle_message(ActorMessage {
                scope: assistant.scope,
                message: agent_message,
            })
            .await;

        // Should remain in Processing
        assert!(matches!(assistant.state, AgentStatus::Processing));
        // Message should be queued
        assert!(assistant.pending_message.has_content());
        assert!(assistant.pending_message.system_parts.iter().any(|part| {
            part.contains(&format!(
                "<sub_agent_message agent_id=\"{}\">Message while processing</sub_agent_message>",
                agent_id
            ))
        }));
    }

    #[tokio::test]
    async fn test_sub_agent_message_while_awaiting_tools() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::AwaitingTools {
            pending_tool_calls: vec!["tool123".to_string()],
        };
        let agent_id = Scope::new();

        let agent_message = Message::Agent(AgentMessage {
            agent_id,
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::SubAgentMessage {
                message: "Message while waiting for tools".to_string(),
            }),
        });

        assistant
            .handle_message(ActorMessage {
                scope: assistant.scope,
                message: agent_message,
            })
            .await;

        // Should remain in AwaitingTools
        assert!(matches!(assistant.state, AgentStatus::AwaitingTools { .. }));
        // Message should be queued
        assert!(assistant.pending_message.has_content());
        assert!(assistant.pending_message.system_parts.iter().any(|part| {
            part.contains(&format!("<sub_agent_message agent_id=\"{}\">Message while waiting for tools</sub_agent_message>", agent_id))
        }));
    }

    #[tokio::test]
    async fn test_multiple_sub_agent_messages_queuing() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AgentStatus::AwaitingTools {
            pending_tool_calls: vec!["tool123".to_string()],
        };
        let agent1_id = Scope::new();
        let agent2_id = Scope::new();

        // Send first message
        let agent_message1 = Message::Agent(AgentMessage {
            agent_id: agent1_id,
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::SubAgentMessage {
                message: "First message".to_string(),
            }),
        });

        assistant
            .handle_message(ActorMessage {
                scope: assistant.scope,
                message: agent_message1,
            })
            .await;

        // Send second message
        let agent_message2 = Message::Agent(AgentMessage {
            agent_id: agent2_id,
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::SubAgentMessage {
                message: "Second message".to_string(),
            }),
        });

        assistant
            .handle_message(ActorMessage {
                scope: assistant.scope,
                message: agent_message2,
            })
            .await;

        // Should remain in AwaitingTools
        assert!(matches!(assistant.state, AgentStatus::AwaitingTools { .. }));
        // Both messages should be queued
        assert!(assistant.pending_message.has_content());
        assert_eq!(assistant.pending_message.system_parts.len(), 2);
        assert!(assistant.pending_message.system_parts.iter().any(|part| {
            part.contains(&format!(
                "<sub_agent_message agent_id=\"{}\">First message</sub_agent_message>",
                agent1_id
            ))
        }));
        assert!(assistant.pending_message.system_parts.iter().any(|part| {
            part.contains(&format!(
                "<sub_agent_message agent_id=\"{}\">Second message</sub_agent_message>",
                agent2_id
            ))
        }));
    }
}

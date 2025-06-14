use genai::{
    Client,
    chat::{ChatMessage, ChatRequest, ChatRole, ContentPart, MessageContent, Tool},
};
use snafu::ResultExt;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, broadcast};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    SResult,
    actors::{Actor, Message, ToolCallStatus, ToolCallUpdate},
    config::ParsedModelConfig,
    system_state::SystemState,
    template::ToolInfo,
};

use super::{
    Action, ActorMessage, AgentMessageType, AgentTaskResult, AgentTaskResultOk, AgentTaskStatus,
    InterAgentMessage, TaskAwaitingManager, UserContext,
};

/// States that the Assistant actor can be in
#[derive(Debug, Clone)]
pub enum AssistantState {
    /// Waiting for actors
    AwaitingActors,
    /// Ready to accept requests, has tools available
    Idle,
    /// Actively processing a user request (making LLM call)
    Processing,
    /// Waiting for tool execution results
    AwaitingTools { pending_tool_calls: Vec<String> },
    /// Waiting for next input from user, sub agent, etc...
    /// Does not submit a response to the LLM when the tool call with `tool_call_id` returns a
    /// response. Waits for other input
    Wait { tool_call_id: String },
    /// Waiting for manager plan approval
    AwaitingManager(TaskAwaitingManager),
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
    pending_tool_responses: Vec<genai::chat::ToolResponse>,
    state: AssistantState,
    task_description: Option<String>,
    scope: Uuid,
    spawned_agents_scope: Vec<Uuid>,
    required_actors: Vec<&'static str>,
    waiting_for_agents: HashMap<Uuid, Option<AgentTaskResult>>,
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
        scope: Uuid,
        required_actors: Vec<&'static str>,
        task_description: Option<String>,
    ) -> Self {
        let client = Client::builder()
            .with_service_target_resolver(config.service_target_resolver.clone())
            .build();

        let state = if required_actors.is_empty() {
            AssistantState::Idle
        } else {
            AssistantState::AwaitingActors
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
            waiting_for_agents: HashMap::new(),
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
        // TODO: This needs to be redone. See the note above the impl
        // Check if this is a completion
        if let ToolCallStatus::Finished(result) = &update.status {
            match &self.state {
                AssistantState::AwaitingTools { pending_tool_calls } => {
                    if pending_tool_calls.contains(&update.call_id) {
                        let mut remaining_calls = pending_tool_calls.clone();
                        remaining_calls.retain(|id| id != &update.call_id);

                        // Create tool response and add to chat
                        let tool_response = genai::chat::ToolResponse {
                            call_id: update.call_id.clone(),
                            content: result.clone().unwrap_or_else(|e| format!("Error: {}", e)),
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

                            // Check if we should transition to Wait state or continue normally
                            // TODO: Handle transitions to Wait and AwaitingManager states here based on tool responses

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
                        } else {
                            // Still waiting for more tools
                            self.state = AssistantState::AwaitingTools {
                                pending_tool_calls: remaining_calls,
                            };
                        }
                    }
                }
                AssistantState::Wait { tool_call_id } => {
                    if tool_call_id == &update.call_id {
                        let tool_response = genai::chat::ToolResponse {
                            call_id: update.call_id.clone(),
                            content: result.clone().unwrap_or_else(|e| format!("Error: {}", e)),
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

    fn add_system_message(&mut self, content: impl Into<MessageContent>) {
        self.chat_request = self
            .chat_request
            .clone()
            .append_message(ChatMessage::system(content));
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

    #[tracing::instrument(name = "llm_request", skip(self))]
    async fn submit_llm_request(&mut self) {
        if !self.pending_tool_responses.is_empty() {
            warn!("Submitting assistant request while pending_tool_responses is not empty");
        }

        self.state = AssistantState::Processing;

        // Cancel any existing request
        if let Some(handle) = self.cancel_handle.lock().await.take() {
            if !handle.is_finished() {
                handle.abort();
                warn!("Implicitly cancelling stale assistant request");
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
                vec![], // Maybe pass the whitelisted commands to the model as well?
                self.task_description.clone(),
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
    scope: Uuid,
) -> SResult<()> {
    let request = chat_request;

    let resp = client
        .exec_chat(&config.name, request, None)
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

    fn get_scope(&self) -> &Uuid {
        &self.scope
    }

    fn get_scope_filters(&self) -> Vec<&Uuid> {
        self.spawned_agents_scope
            .iter()
            .chain([&self.scope])
            .collect::<Vec<&Uuid>>()
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
                    if let AssistantState::AwaitingActors = &self.state {
                        self.required_actors = self
                            .required_actors
                            .drain(..)
                            .filter(|r_id| r_id != &actor_id.as_str())
                            .collect::<Vec<&'static str>>();

                        if self.required_actors.is_empty() {
                            self.state = AssistantState::Idle;

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
                    // State transition: Idle -> Processing when receiving user input
                    if let AssistantState::Idle = &self.state {
                        self.state = AssistantState::Processing;
                    }

                    match (context, &self.state) {
                        (UserContext::UserTUIInput(text), AssistantState::Idle) => {
                            self.add_user_content(ContentPart::Text(text));
                            self.submit_pending_message().await;
                        }
                        (UserContext::UserTUIInput(text), _) => {
                            self.add_user_content(ContentPart::Text(text));
                        }

                        #[cfg(feature = "audio")]
                        (UserContext::MicrophoneTranscription(text), _) => {
                            self.add_user_content(ContentPart::Text(text));
                        }
                        #[cfg(feature = "gui")]
                        (UserContext::ScreenshotCaptured(result), _) => {
                            if let Ok(base64) = result {
                                // Add screenshot as an image content part
                                let content_part = genai::chat::ContentPart::from_image_base64(
                                    "image/png",
                                    base64,
                                );
                                self.add_user_content(content_part);
                            }
                            // Errors are already handled by TUI
                        }
                        #[cfg(feature = "gui")]
                        (UserContext::ClipboardCaptured(_result), _) => {
                            // Clipboard text is sent as UserTUIInput by the TUI actor when the user hits
                            // enter so we don't need to handle it here
                        }
                    }
                }

                Message::Action(crate::actors::Action::Assist) => {
                    // State transition: Idle -> Processing when receiving assist action
                    if let AssistantState::Idle = &self.state {
                        self.submit_pending_message().await;
                    }
                }
                Message::Action(crate::actors::Action::Cancel) => {
                    // State transition: Processing/AwaitingTools -> Idle on cancel
                    match &self.state {
                        AssistantState::Processing | AssistantState::AwaitingTools { .. } => {
                            self.state = AssistantState::Idle;
                            // If we have pending message content, submit it immediately
                            if self.pending_message.has_content() {
                                self.submit_pending_message().await;
                            }
                        }
                        _ => {}
                    }
                    // Cancel current request
                    if let Some(handle) = self.cancel_handle.lock().await.take() {
                        handle.abort();
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
                    // State transition: Processing -> AwaitingTools or Idle based on content
                    if let AssistantState::Processing = &self.state {
                        match &content {
                            MessageContent::ToolCalls(tool_calls) => {
                                let call_ids =
                                    tool_calls.iter().map(|tc| tc.call_id.clone()).collect();
                                self.state = AssistantState::AwaitingTools {
                                    pending_tool_calls: call_ids,
                                };
                            }
                            _ => {
                                self.state = AssistantState::Idle;
                                // If we have pending message content, submit it immediately
                                if self.pending_message.has_content() {
                                    self.submit_pending_message().await;
                                }
                            }
                        }
                    }

                    self.chat_request = self.chat_request.clone().append_message(ChatMessage {
                        role: ChatRole::Assistant,
                        content,
                        options: None,
                    });
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
                        if let AssistantState::Wait {
                            tool_call_id: waiting_for_tool_call_id,
                        } = &self.state
                        {
                            if waiting_for_tool_call_id == &tool_call_id {
                                self.waiting_for_agents.insert(message.agent_id, None);
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
                                match &status {
                                    super::AgentTaskStatus::Done(_) => (),
                                    super::AgentTaskStatus::InProgress => (),
                                    super::AgentTaskStatus::AwaitingManager(
                                        task_awaiting_manager,
                                    ) => {
                                        self.state = AssistantState::AwaitingManager(
                                            task_awaiting_manager.clone(),
                                        );
                                    }
                                    super::AgentTaskStatus::Waiting { tool_call_id } => {
                                        self.state = AssistantState::Wait {
                                            tool_call_id: tool_call_id.clone(),
                                        }
                                    }
                                }
                                self.system_state
                                    .update_agent_status(&message.agent_id, status);
                            }
                            InterAgentMessage::PlanApproved { plan_id: _ } => (),
                            InterAgentMessage::PlanRejected {
                                plan_id: _,
                                reason: _,
                            } => (),
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
                                    (_, AssistantState::AwaitingActors) => {
                                        error!(
                                            "Received sub-agent status update while awaiting actors - this should not happen"
                                        );
                                    }

                                    // Sub-agent Done status handling
                                    (
                                        AgentTaskStatus::Done(agent_task_result),
                                        AssistantState::Wait { tool_call_id: _ },
                                    ) => {
                                        // We're waiting for this agent - track completion
                                        match self.waiting_for_agents.get_mut(&message.agent_id) {
                                            Some(opt) => *opt = Some(agent_task_result.clone()),
                                            None => warn!(
                                                "Received completion from sub-agent we aren't waiting for: {}",
                                                message.agent_id
                                            ),
                                        }

                                        // Check if all agents we're waiting for are done
                                        if self.waiting_for_agents.values().all(|x| x.is_some()) {
                                            // All agents done - compile their responses
                                            let agent_summaries = self.waiting_for_agents.drain().map(|(agent_id, agent_result)| {
                                                match agent_result.unwrap() {
                                                    Ok(res) => format!("<agent_response id={agent_id}>status: {}\n\n{}</agent_response>", 
                                                        if res.success { "SUCCESS" } else { "FAILURE" }, res.summary),
                                                    Err(err) => format!("<agent_response id={agent_id}>status: FAILURE\n\n{err}</agent_response>"),
                                                }
                                            }).collect::<Vec<String>>();
                                            let summary_text = agent_summaries.join("\n\n");

                                            // Add to pending message and submit
                                            self.add_system_message_part(summary_text);
                                            self.state = AssistantState::Processing;
                                            self.submit_pending_message().await;
                                        } else {
                                            // Still waiting for other agents - add this completion to pending
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
                                    }

                                    (
                                        AgentTaskStatus::Done(agent_task_result),
                                        AssistantState::Idle,
                                    ) => {
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
                                        AgentTaskStatus::Done(agent_task_result),
                                        AssistantState::Processing
                                        | AssistantState::AwaitingTools { .. }
                                        | AssistantState::AwaitingManager(_),
                                    ) => {
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

                                    // Sub-agent AwaitingManager status handling
                                    (
                                        AgentTaskStatus::AwaitingManager(task_awaiting_manager),
                                        AssistantState::Wait { tool_call_id: _ },
                                    ) => {
                                        // Urgent: Plan approval needed - transition out of Wait and submit immediately
                                        let approval_request = format!(
                                            "<plan_approval_request agent_id={}>\n{}</plan_approval_request>",
                                            message.agent_id,
                                            serde_json::to_string_pretty(task_awaiting_manager)
                                                .unwrap_or_else(
                                                    |_| "Failed to serialize plan".to_string()
                                                )
                                        );
                                        self.add_system_message_part(approval_request);
                                        self.state = AssistantState::Processing;
                                        self.submit_pending_message().await;
                                    }

                                    (
                                        AgentTaskStatus::AwaitingManager(task_awaiting_manager),
                                        AssistantState::Idle,
                                    ) => {
                                        // Add plan approval request to pending message and submit
                                        let approval_request = format!(
                                            "<plan_approval_request agent_id={}>\n{}</plan_approval_request>",
                                            message.agent_id,
                                            serde_json::to_string_pretty(task_awaiting_manager)
                                                .unwrap_or_else(
                                                    |_| "Failed to serialize plan".to_string()
                                                )
                                        );
                                        self.add_system_message_part(approval_request);
                                        self.submit_pending_message().await;
                                    }

                                    (
                                        AgentTaskStatus::AwaitingManager(task_awaiting_manager),
                                        AssistantState::Processing
                                        | AssistantState::AwaitingTools { .. }
                                        | AssistantState::AwaitingManager(_),
                                    ) => {
                                        // Add plan approval request to pending message for later submission
                                        let approval_request = format!(
                                            "<plan_approval_request agent_id={}>\n{}</plan_approval_request>",
                                            message.agent_id,
                                            serde_json::to_string_pretty(task_awaiting_manager)
                                                .unwrap_or_else(
                                                    |_| "Failed to serialize plan".to_string()
                                                )
                                        );
                                        self.add_system_message_part(approval_request);
                                    }

                                    // Sub-agent InProgress status - just update system state, no action
                                    (AgentTaskStatus::InProgress, _) => {
                                        // Update system state only - no pending message needed
                                    }

                                    // Sub-agent Waiting status - update system state only
                                    (AgentTaskStatus::Waiting { .. }, _) => {
                                        // Update system state only - no pending message needed
                                    }
                                }
                            }
                            InterAgentMessage::PlanApproved { plan_id: _ } => (),
                            InterAgentMessage::PlanRejected {
                                plan_id: _,
                                reason: _,
                            } => (),
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
    // - Wait: Waiting for spawned sub-agents to complete (tool_call_id tracks the tool that caused wait)
    // - AwaitingManager: WE are waiting for OUR manager's approval on OUR plan
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
    // 2. Idle -> Processing (on UserContext or Action::Assist):
    //    - UserContext: Add to pending message, submit LLM request
    //    - Action::Assist: Submit pending message to LLM (if any content exists)
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
    //    - Track spawned agents for completion
    //    - Transition to Wait state with tool_call_id
    //
    // 8. AwaitingTools -> AwaitingManager (when tool completes and sets awaiting manager state):
    //    - Add tool response to chat history
    //    - Transition to AwaitingManager
    //    - Wait for our manager's approval/rejection
    //
    // 9. Processing/AwaitingTools -> Idle (on Action::Cancel):
    //    - Cancel current LLM request
    //    - Transition to Idle
    //    - If pending message exists: submit immediately (-> Processing)
    //
    // 10. Wait -> Processing (on sub-agent Done status when no more agents waiting):
    //     - Add agent completion summary to pending message (system part)
    //     - Submit LLM request immediately
    //     - Transition to Processing
    //
    // 11. Wait -> Wait (on sub-agent Done status when still waiting on other agents):
    //     - Add agent completion summary to pending message (system part)
    //     - Update waiting agents tracking
    //     - Remain in Wait state
    //
    // 12. Any state (on sub-agent Done status, not in Wait):
    //     - Add agent completion summary to pending message (system part)
    //     - If currently Idle: submit immediately (-> Processing)
    //     - If currently Processing/AwaitingTools: will submit after current operation completes
    //     - State remains unchanged
    //
    // 13. Any state (on sub-agent AwaitingManager status):
    //     - Add plan approval request to pending message (system part) for US to review as manager
    //     - If currently Idle: submit immediately (-> Processing)
    //     - If currently Processing/AwaitingTools: will submit after current operation completes
    //     - If currently Wait: submit immediately (-> Processing) and transition out of Wait
    //     - Other states remain unchanged (WE don't transition to AwaitingManager - they are awaiting US)
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

    fn create_test_assistant(
        required_actors: Vec<&'static str>,
        task_description: Option<String>,
    ) -> Assistant {
        use crate::config::Config;

        let config = Config::default().unwrap();
        let parsed_config: ParsedConfig = config.try_into().unwrap();
        let parsed_config = parsed_config.hive.main_manager_model;

        let (tx, _) = broadcast::channel(10);
        let scope = Uuid::new_v4();
        Assistant::new(parsed_config, tx, scope, required_actors, task_description)
    }

    async fn send_message(assistant: &mut Assistant, message: Message) {
        let actor_message = ActorMessage {
            scope: assistant.scope.clone(),
            message,
        };
        assistant.handle_message(actor_message).await;
    }

    #[test]
    fn test_initial_state_with_required_actors() {
        let assistant = create_test_assistant(vec!["tool1", "tool2"], None);
        assert!(matches!(assistant.state, AssistantState::AwaitingActors));
    }

    #[test]
    fn test_initial_state_without_required_actors() {
        let assistant = create_test_assistant(vec![], None);
        assert!(matches!(assistant.state, AssistantState::Idle));
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
        assert!(matches!(assistant.state, AssistantState::AwaitingActors));

        // Send second ActorReady - should transition to Idle
        send_message(
            &mut assistant,
            Message::ActorReady {
                actor_id: "tool2".to_string(),
            },
        )
        .await;
        assert!(matches!(assistant.state, AssistantState::Idle));
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
        assert!(matches!(assistant.state, AssistantState::Processing));

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
        assert!(matches!(assistant.state, AssistantState::Idle));

        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Hello".to_string())),
        )
        .await;

        assert!(matches!(assistant.state, AssistantState::Processing));
    }

    #[tokio::test]
    async fn test_idle_to_processing_on_assist_action() {
        let mut assistant = create_test_assistant(vec![], None);
        assert!(matches!(assistant.state, AssistantState::Idle));

        // Add some content to the pending message first
        assistant.add_user_content(ContentPart::Text("Hello".to_string()));

        send_message(
            &mut assistant,
            Message::Action(crate::actors::Action::Assist),
        )
        .await;

        assert!(matches!(assistant.state, AssistantState::Processing));
    }

    #[tokio::test]
    async fn test_assist_action_with_no_content() {
        let mut assistant = create_test_assistant(vec![], None);
        assert!(matches!(assistant.state, AssistantState::Idle));

        send_message(
            &mut assistant,
            Message::Action(crate::actors::Action::Assist),
        )
        .await;

        // Should remain in Idle state since there's no content to submit
        assert!(matches!(assistant.state, AssistantState::Idle));
    }

    #[tokio::test]
    async fn test_processing_to_idle_on_text_response() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AssistantState::Processing;

        send_message(
            &mut assistant,
            Message::AssistantResponse(MessageContent::Text("Response".to_string())),
        )
        .await;

        assert!(matches!(assistant.state, AssistantState::Idle));
    }

    #[tokio::test]
    async fn test_processing_to_awaiting_tools_on_tool_calls() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AssistantState::Processing;

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
            AssistantState::AwaitingTools { pending_tool_calls } => {
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
        assistant.state = AssistantState::AwaitingTools {
            pending_tool_calls: vec!["call_123".to_string()],
        };

        let update = ToolCallUpdate {
            call_id: "call_123".to_string(),
            status: ToolCallStatus::Finished(Ok("Success".to_string())),
        };

        send_message(&mut assistant, Message::ToolCallUpdate(update)).await;

        assert!(matches!(assistant.state, AssistantState::Processing));
    }

    #[tokio::test]
    async fn test_awaiting_tools_partial_completion() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AssistantState::AwaitingTools {
            pending_tool_calls: vec!["call_123".to_string(), "call_456".to_string()],
        };

        let update = ToolCallUpdate {
            call_id: "call_123".to_string(),
            status: ToolCallStatus::Finished(Ok("Success".to_string())),
        };

        send_message(&mut assistant, Message::ToolCallUpdate(update)).await;

        match &assistant.state {
            AssistantState::AwaitingTools { pending_tool_calls } => {
                assert_eq!(pending_tool_calls.len(), 1);
                assert!(pending_tool_calls.contains(&"call_456".to_string()));
            }
            _ => panic!("Expected AwaitingTools state"),
        }
    }

    #[tokio::test]
    async fn test_cancel_from_processing_to_idle() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AssistantState::Processing;

        send_message(
            &mut assistant,
            Message::Action(crate::actors::Action::Cancel),
        )
        .await;

        assert!(matches!(assistant.state, AssistantState::Idle));
    }

    #[tokio::test]
    async fn test_cancel_from_awaiting_tools_to_idle() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AssistantState::AwaitingTools {
            pending_tool_calls: vec!["call_123".to_string()],
        };

        send_message(
            &mut assistant,
            Message::Action(crate::actors::Action::Cancel),
        )
        .await;

        assert!(matches!(assistant.state, AssistantState::Idle));
    }

    #[tokio::test]
    async fn test_no_transition_user_input_while_processing() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AssistantState::Processing;

        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Ignored".to_string())),
        )
        .await;

        // Should remain in Processing state
        assert!(matches!(assistant.state, AssistantState::Processing));
    }

    #[tokio::test]
    async fn test_no_transition_user_input_while_awaiting_tools() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AssistantState::AwaitingTools {
            pending_tool_calls: vec!["call_123".to_string()],
        };

        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Ignored".to_string())),
        )
        .await;

        // Should remain in AwaitingTools state
        assert!(matches!(
            assistant.state,
            AssistantState::AwaitingTools { .. }
        ));
    }

    #[tokio::test]
    async fn test_no_transition_wrong_tool_call_id() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AssistantState::AwaitingTools {
            pending_tool_calls: vec!["call_123".to_string()],
        };

        let update = ToolCallUpdate {
            call_id: "wrong_call_id".to_string(),
            status: ToolCallStatus::Finished(Ok("Success".to_string())),
        };

        send_message(&mut assistant, Message::ToolCallUpdate(update)).await;

        // Should remain in AwaitingTools state with same pending calls
        match &assistant.state {
            AssistantState::AwaitingTools { pending_tool_calls } => {
                assert_eq!(pending_tool_calls.len(), 1);
                assert!(pending_tool_calls.contains(&"call_123".to_string()));
            }
            _ => panic!("Expected AwaitingTools state"),
        }
    }

    #[tokio::test]
    async fn test_no_transition_actor_ready_when_idle() {
        let mut assistant = create_test_assistant(vec![], None);
        assert!(matches!(assistant.state, AssistantState::Idle));

        send_message(
            &mut assistant,
            Message::ActorReady {
                actor_id: "unexpected".to_string(),
            },
        )
        .await;

        // Should remain in Idle state
        assert!(matches!(assistant.state, AssistantState::Idle));
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

    // Helper function to create agent messages
    fn create_agent_message(agent_id: Uuid, status: AgentTaskStatus) -> ActorMessage {
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

    #[tokio::test]
    async fn test_sub_agent_done_in_idle_state() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AssistantState::Idle;

        let agent_id = Uuid::new_v4();
        let task_result = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Task completed".to_string(),
        });

        let agent_message = create_agent_message(agent_id, AgentTaskStatus::Done(task_result));
        assistant.handle_message(agent_message).await;

        // Should transition to Processing after receiving sub-agent completion
        assert!(matches!(assistant.state, AssistantState::Processing));

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
        assistant.state = AssistantState::Processing;

        let agent_id = Uuid::new_v4();
        let task_result = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Task completed".to_string(),
        });

        let agent_message = create_agent_message(agent_id, AgentTaskStatus::Done(task_result));
        assistant.handle_message(agent_message).await;

        // Should remain in Processing state
        assert!(matches!(assistant.state, AssistantState::Processing));

        // Should have added system message to pending message for later submission
        assert!(!assistant.pending_message.system_parts.is_empty());
    }

    #[tokio::test]
    async fn test_sub_agent_awaiting_manager_in_idle() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AssistantState::Idle;

        let agent_id = Uuid::new_v4();
        let task_awaiting = TaskAwaitingManager::AwaitingMoreInformation("test plan".to_string());

        let agent_message =
            create_agent_message(agent_id, AgentTaskStatus::AwaitingManager(task_awaiting));
        assistant.handle_message(agent_message).await;

        // Should transition to Processing for plan approval
        assert!(matches!(assistant.state, AssistantState::Processing));

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
        assistant.state = AssistantState::Wait {
            tool_call_id: "tool123".to_string(),
        };

        let agent_id = Uuid::new_v4();
        let task_awaiting = TaskAwaitingManager::AwaitingMoreInformation("test plan".to_string());

        let agent_message =
            create_agent_message(agent_id, AgentTaskStatus::AwaitingManager(task_awaiting));
        assistant.handle_message(agent_message).await;

        // Should transition out of Wait to Processing for urgent plan approval
        assert!(matches!(assistant.state, AssistantState::Processing));

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
        let agent_id = Uuid::new_v4();

        // Set up Wait state with one agent
        assistant.state = AssistantState::Wait {
            tool_call_id: "tool123".to_string(),
        };
        assistant.waiting_for_agents.insert(agent_id, None);

        let task_result = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Task completed".to_string(),
        });

        let agent_message = create_agent_message(agent_id, AgentTaskStatus::Done(task_result));
        assistant.handle_message(agent_message).await;

        // Should transition to Processing since all agents are done
        assert!(matches!(assistant.state, AssistantState::Processing));

        // waiting_for_agents should be cleared
        assert!(assistant.waiting_for_agents.is_empty());

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
        let agent1_id = Uuid::new_v4();
        let agent2_id = Uuid::new_v4();

        // Set up Wait state with two agents
        assistant.state = AssistantState::Wait {
            tool_call_id: "tool123".to_string(),
        };
        assistant.waiting_for_agents.insert(agent1_id, None);
        assistant.waiting_for_agents.insert(agent2_id, None);

        let task_result = Ok(crate::actors::AgentTaskResultOk {
            success: true,
            summary: "Agent 1 completed".to_string(),
        });

        // First agent completes
        let agent_message = create_agent_message(agent1_id, AgentTaskStatus::Done(task_result));
        assistant.handle_message(agent_message).await;

        // Should remain in Wait state since agent2 is still pending
        assert!(matches!(assistant.state, AssistantState::Wait { .. }));

        // Should have one completed agent and one pending
        assert_eq!(assistant.waiting_for_agents.len(), 2);
        assert!(
            assistant
                .waiting_for_agents
                .get(&agent1_id)
                .unwrap()
                .is_some()
        );
        assert!(
            assistant
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
        assistant.state = AssistantState::Idle;

        let agent_id = Uuid::new_v4();
        let agent_message = create_agent_message(agent_id, AgentTaskStatus::InProgress);
        assistant.handle_message(agent_message).await;

        // Should remain in same state
        assert!(matches!(assistant.state, AssistantState::Idle));

        // Should not add anything to pending message
        assert!(!assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_sub_agent_waiting_no_action() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AssistantState::Idle;

        let agent_id = Uuid::new_v4();
        let agent_message = create_agent_message(
            agent_id,
            AgentTaskStatus::Waiting {
                tool_call_id: "tool123".to_string(),
            },
        );
        assistant.handle_message(agent_message).await;

        // Should remain in same state
        assert!(matches!(assistant.state, AssistantState::Idle));

        // Should not add anything to pending message
        assert!(!assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_user_input_accumulates_in_pending_message() {
        let mut assistant = create_test_assistant(vec![], None);
        assistant.state = AssistantState::Processing;

        // Add user input while processing
        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Hello".to_string())),
        )
        .await;

        // Should remain in Processing state
        assert!(matches!(assistant.state, AssistantState::Processing));

        // Should accumulate in pending message
        assert!(assistant.pending_message.has_content());
        assert!(assistant.pending_message.user_content.is_some());
    }
}

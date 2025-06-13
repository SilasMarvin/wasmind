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
    /// Encountered an error during processing
    Error { message: String },
    /// Waiting for next input from user, sub agent, etc...
    /// Does not submit a response to the LLM when the tool call with `tool_call_id` returns a
    /// response. Waits for other input
    Wait { tool_call_id: String },
    /// Waiting for manager plan approval
    AwaitingManager(TaskAwaitingManager),
}

// TODO: Add some kind of message queue system for the assistant
// This will be used when we get messages from sub agents or the user while performing other tasks

/// Assistant actor that handles AI interactions
pub struct Assistant {
    tx: broadcast::Sender<ActorMessage>,
    config: ParsedModelConfig,
    client: Client,
    chat_request: ChatRequest,
    system_state: SystemState,
    available_tools: Vec<Tool>,
    cancel_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pending_user_content_parts: Vec<genai::chat::ContentPart>,
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
            pending_user_content_parts: Vec::new(),
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
                            // Automatically continue the conversation
                            self.chat_request =
                                self.chat_request.clone().append_message(ChatMessage {
                                    role: ChatRole::Tool,
                                    content: MessageContent::ToolResponses(std::mem::take(
                                        &mut self.pending_tool_responses,
                                    )),
                                    options: None,
                                });
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

    fn add_user_pending_part(&mut self, content: ContentPart) {
        self.pending_user_content_parts.push(content);
    }

    fn add_system_message(&mut self, content: impl Into<MessageContent>) {
        self.chat_request = self
            .chat_request
            .clone()
            .append_message(ChatMessage::system(content));
    }

    async fn submit_assist_request(&mut self) {
        self.chat_request =
            self.chat_request
                .clone()
                .append_message(ChatMessage::user(MessageContent::Parts(std::mem::take(
                    &mut self.pending_user_content_parts,
                ))));
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
            "LLM Request:\n{}",
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
        "LLM Response: content={:?}, reasoning_content={:?}, usage={:?}, model={}",
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
                        info!("GOT ACTOR READY: {}", actor_id);
                        self.required_actors = self
                            .required_actors
                            .drain(..)
                            .filter(|r_id| r_id != &actor_id.as_str())
                            .collect::<Vec<&'static str>>();

                        info!("REQUIRED ACTORS LEFT: {:?}", self.required_actors);

                        if self.required_actors.is_empty() {
                            self.state = AssistantState::Idle;

                            // Check if we have a task to execute
                            if let Some(ref task) = self.task_description {
                                self.add_user_pending_part(ContentPart::Text(task.clone()));
                                self.submit_assist_request().await;
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
                            self.add_user_pending_part(ContentPart::Text(text));
                            self.submit_assist_request().await;
                        }
                        (UserContext::UserTUIInput(text), _) => {
                            self.add_user_pending_part(ContentPart::Text(text));
                        }

                        #[cfg(feature = "audio")]
                        (UserContext::MicrophoneTranscription(text), _) => {
                            self.add_user_pending_part(ContentPart::Text(text));
                        }
                        #[cfg(feature = "gui")]
                        (UserContext::ScreenshotCaptured(result), _) => {
                            if let Ok(base64) = result {
                                // Add screenshot as an image content part
                                let content_part = genai::chat::ContentPart::from_image_base64(
                                    "image/png",
                                    base64,
                                );
                                self.add_user_pending_part(content_part);
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
                        self.submit_assist_request().await;
                    }
                }
                Message::Action(crate::actors::Action::Cancel) => {
                    // State transition: Processing/AwaitingTools -> Idle on cancel
                    match &self.state {
                        AssistantState::Processing | AssistantState::AwaitingTools { .. } => {
                            self.state = AssistantState::Idle;
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
                                // TODO: Map out all of the transitions and what should happen here
                                // Every single one of these needs to be well thought through.
                                // Adding a message queue is a prerequisite to fully completing
                                // this.
                                match (status, &self.state) {
                                    (
                                        AgentTaskStatus::Done(agent_task_result),
                                        AssistantState::AwaitingActors,
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::Done(agent_task_result),
                                        AssistantState::Idle,
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::Done(agent_task_result),
                                        AssistantState::Processing,
                                    ) => todo!(),
                                    // This call happens when the complete tool is used
                                    // We are waiting for the complete tool call and the complete
                                    // tool call updates our AgentTaskStatus to Done
                                    (
                                        AgentTaskStatus::Done(agent_task_result),
                                        AssistantState::AwaitingTools { pending_tool_calls },
                                    ) => (),
                                    (
                                        AgentTaskStatus::Done(agent_task_result),
                                        AssistantState::Error { message },
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::Done(agent_task_result),
                                        AssistantState::Wait { tool_call_id: _ },
                                    ) => {
                                        match self.waiting_for_agents.get_mut(&message.agent_id) {
                                            Some(opt) => *opt = Some(agent_task_result),
                                            None => warn!(
                                                "Received a response from a sub agent we aren't waiting on while actively waiting"
                                            ),
                                        }
                                        if self.waiting_for_agents.values().all(|x| x.is_some()) {
                                            let text = self.waiting_for_agents.drain().map(|(agent_id, agent_result)| {
                                                match agent_result.unwrap() {
                                                    Ok(res) => format!("<agent_response id={agent_id}>status: {}\n\n{}</agent_response>", if res.success { "SUCCESS" } else { "FAILURE" }, res.summary),
                                                    Err(err) => format!("<agent_response id={agent_id}>status: FAILURE\n\n{err}</agent_response>"),
                                                }
                                            }).collect::<Vec<String>>();
                                            let text = text.join("\n\n");
                                            self.add_system_message(&text);

                                            self.submit_llm_request().await;
                                        }
                                    }
                                    (
                                        AgentTaskStatus::Done(agent_task_result),
                                        AssistantState::AwaitingManager(task_awaiting_manager),
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::InProgress,
                                        AssistantState::AwaitingActors,
                                    ) => todo!(),
                                    (AgentTaskStatus::InProgress, AssistantState::Idle) => todo!(),
                                    (AgentTaskStatus::InProgress, AssistantState::Processing) => {
                                        todo!()
                                    }
                                    (
                                        AgentTaskStatus::InProgress,
                                        AssistantState::AwaitingTools { pending_tool_calls },
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::InProgress,
                                        AssistantState::Error { message },
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::InProgress,
                                        AssistantState::Wait { tool_call_id },
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::InProgress,
                                        AssistantState::AwaitingManager(task_awaiting_manager),
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::AwaitingManager(task_awaiting_manager),
                                        AssistantState::AwaitingActors,
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::AwaitingManager(task_awaiting_manager),
                                        AssistantState::Idle,
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::AwaitingManager(task_awaiting_manager),
                                        AssistantState::Processing,
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::AwaitingManager(task_awaiting_manager),
                                        AssistantState::AwaitingTools { pending_tool_calls },
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::AwaitingManager(task_awaiting_manager),
                                        AssistantState::Error { message },
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::AwaitingManager(task_awaiting_manager),
                                        AssistantState::Wait { tool_call_id },
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::AwaitingManager(task_awaiting_manager),
                                        AssistantState::AwaitingManager(task_awaiting_manager_),
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::Waiting { tool_call_id },
                                        AssistantState::AwaitingActors,
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::Waiting { tool_call_id },
                                        AssistantState::Idle,
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::Waiting { tool_call_id },
                                        AssistantState::Processing,
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::Waiting { tool_call_id },
                                        AssistantState::AwaitingTools { pending_tool_calls },
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::Waiting { tool_call_id },
                                        AssistantState::Error { message },
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::Waiting { tool_call_id },
                                        AssistantState::Wait { tool_call_id: _ },
                                    ) => todo!(),
                                    (
                                        AgentTaskStatus::Waiting { tool_call_id },
                                        AssistantState::AwaitingManager(task_awaiting_manager),
                                    ) => todo!(),
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

    // Assistant State Transitions Documentation
    // ========================================
    //
    // The Assistant actor has the following states:
    // - AwaitingActors: Initial state when required actors are not ready
    // - Idle: Ready to accept requests
    // - Processing: Actively making an LLM call
    // - AwaitingTools: Waiting for tool execution results
    // - Error: Error state (currently unused in transitions)
    // - Wait: Waiting for external input (currently unused in transitions)
    // - AwaitingManager: Waiting for manager approval (currently unused in transitions)
    //
    // Valid State Transitions:
    // 1. AwaitingActors -> Idle: When all required actors send ActorReady messages
    //    - Special case: If task_description exists, immediately processes the task
    // 2. Idle -> Processing: When receiving UserContext (UserTUIInput) or Action::Assist
    // 3. Processing -> Idle: When AssistantResponse contains no tool calls
    // 4. Processing -> AwaitingTools: When AssistantResponse contains tool calls
    // 5. AwaitingTools -> Processing: When all pending tool calls are finished
    // 6. AwaitingTools -> AwaitingTools: When some tool calls finish but others remain
    // 7. Processing -> Idle: When Action::Cancel is received
    // 8. AwaitingTools -> Idle: When Action::Cancel is received
    //
    // Messages that don't cause transitions:
    // - UserContext while in Processing or AwaitingTools states
    // - ToolCallUpdate for unknown call_id or when not in AwaitingTools state
    // - ActorReady when not in AwaitingActors state
    // - Most other messages don't affect state

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
        let last_message = ChatMessage::user("Test task");
        assert!(matches!(messages, last_message));
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

        send_message(
            &mut assistant,
            Message::Action(crate::actors::Action::Assist),
        )
        .await;

        assert!(matches!(assistant.state, AssistantState::Processing));
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
}

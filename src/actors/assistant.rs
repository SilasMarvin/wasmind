use genai::{
    Client,
    chat::{ChatMessage, ChatRequest, ChatRole, ContentPart, MessageContent, Tool, ToolResponse},
};
use snafu::ResultExt;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tracing::error;
use uuid::Uuid;

use crate::{
    SResult,
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
                    s.pending_message
                        .set_user_content(ContentPart::Text(s.task_description.take().unwrap()));
                    s.submit_pending_message(false);
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
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status: self.state.clone(),
                }),
            }),
        });
    }

    /// Update state and broadcast the change
    fn set_state(&mut self, new_state: AgentStatus) {
        self.state = new_state;
        self.broadcast_state_change();
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

    fn handle_tool_call_update(&mut self, update: ToolCallUpdate) {
        if let AgentStatus::Wait {
            reason: WaitReason::WaitingForTools { tool_calls },
        } = &mut self.state
        {
            if let ToolCallStatus::Finished(result) = &update.status {
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
        self.set_state(AgentStatus::Processing {
            id: processing_id.clone(),
        });

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
                                    match &self.state {
                                        AgentStatus::Wait { reason } => match reason {
                                            WaitReason::WaitingForUserInput
                                            | WaitReason::WaitForDuration { .. }
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
                                self.set_state(AgentStatus::Wait {
                                    reason: WaitReason::WaitingForUserInput,
                                });
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
                                self.set_state(AgentStatus::Wait {
                                    reason: WaitReason::WaitingForTools { tool_calls },
                                });
                            }
                            _ => {
                                // If we have pending message content, submit it immediately when Idle
                                if self.pending_message.has_content() {
                                    self.submit_pending_message(false);
                                } else {
                                    self.set_state(AgentStatus::Wait {
                                        reason: WaitReason::WaitingForUserInput,
                                    });
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
                            InterAgentMessage::TaskStatusUpdate { status } => {
                                // Tool calls may update us to WaitForDuration or
                                // WaitingForPlanApproval
                                match &status {
                                    AgentStatus::Wait { reason } => {
                                        if let AgentStatus::Wait {
                                            reason: WaitReason::WaitingForTools { tool_calls },
                                        } = &self.state
                                        {
                                            match reason {
                                                r @ WaitReason::WaitForDuration {
                                                    tool_call_id,
                                                    timestamp,
                                                    duration,
                                                } => {
                                                    if !tool_calls.contains_key(tool_call_id) {
                                                        return;
                                                    }
                                                    self.set_state(AgentStatus::Wait {
                                                        reason: r.clone(),
                                                    });
                                                }
                                                r @ WaitReason::WaitingForPlanApproval {
                                                    tool_call_id,
                                                } => {
                                                    if !tool_calls.contains_key(tool_call_id) {
                                                        return;
                                                    }
                                                    self.set_state(AgentStatus::Wait {
                                                        reason: r.clone(),
                                                    });
                                                }
                                                _ => (),
                                            }
                                        }
                                    }
                                    _ => (),
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
                            InterAgentMessage::TaskStatusUpdate { status } => {
                                self.system_state
                                    .update_agent_status(&agent_message.agent_id, status.clone());
                            }
                            InterAgentMessage::Message {
                                message: sub_agent_message,
                            } if agent_message.agent_id == self.scope => {
                                let formatted_message =
                                    format_sub_agent_message(&sub_agent_message, &message.scope);
                                self.add_system_message_part(formatted_message);
                                match &self.state {
                                    AgentStatus::Wait { reason } => match reason {
                                        WaitReason::WaitingForUserInput
                                        | WaitReason::WaitForDuration { .. }
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
}

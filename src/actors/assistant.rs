use crate::llm_client::{AssistantChatMessage, ChatMessage, LLMClient, LLMError, Tool};
use snafu::whatever;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;
use tracing::error;
use uuid::Uuid;

use crate::{
    actors::{Actor, AssistantRequest, Message, PendingToolCall, ToolCallStatus, ToolCallUpdate},
    config::ParsedModelConfig,
    scope::Scope,
    system_state::SystemState,
    template::ToolInfo,
};

use super::{
    Action, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, InterAgentMessage,
    UserContext, WaitReason, tools::wait::WAIT_TOOL_NAME,
};

/// Helper functions for formatting messages used in chat requests and tests

const ATTEMPTED_WAIT_ERROR_MESSAGE: &'static str =
    "ERROR: Attempted to wait when all sub agents are done. You must perform a different action.";

/// Format an agent response for successful task completion
pub fn format_agent_response_success(agent_id: &Scope, success: bool, summary: &str) -> String {
    format!(
        "<sub_agent_complete id={}>status: {}\n\n{}</sub_agent_complete>",
        agent_id,
        if success { "SUCCESS" } else { "FAILURE" },
        summary
    )
}

/// Format an agent response for failed task completion
pub fn format_agent_response_failure(agent_id: &Scope, error: &str) -> String {
    format!(
        "<sub_agent_complete id={}>status: FAILURE\n\n{}</sub_agent_complete>",
        agent_id, error
    )
}

/// Format an error message
pub fn format_error_message(error: &impl std::fmt::Display) -> String {
    format!("Error: {}", error)
}

/// Format sub agent message
pub fn format_sub_agent_message(message: &str, agent_id: &Scope) -> String {
    format!(
        r#"New message from one of your sub agents. You can respond using the `send_message` tool.\n<sub_agent_message agent_id="{agent_id}">{message}</sub_agent_message>"#
    )
}

/// Format manager message
pub fn format_manager_message(message: &str) -> String {
    format!(
        r#"New message from your manager. You can respond with the `send_manager_message` tool.\n<manager_message>{message}</manager_message>"#
    )
}

/// Format a general hive system message
pub fn format_general_message(message: &str) -> String {
    format!("**HIVE SYSTEM ALERT**:\n{message}")
}

/// Pending message that accumulates user input and system messages
/// to be submitted to the LLM when appropriate
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

/// Assistant actor that handles AI interactions
pub struct Assistant {
    tx: broadcast::Sender<ActorMessage>,
    config: ParsedModelConfig,
    client: LLMClient,
    chat_history: Vec<ChatMessage>,
    system_state: SystemState,
    available_tools: Vec<Tool>,
    cancel_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pending_message: PendingMessage,
    task_description: Option<String>,
    scope: Scope,
    parent_scope: Scope,
    live_spawned_agents_scope: BTreeSet<Scope>,
    /// Whitelisted commands for the system prompt
    whitelisted_commands: Vec<String>,
    state: AgentStatus,
    /// Agent's role (e.g., "Software Engineer", "QA Tester", "Project Lead Manager")
    role: String,
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
        required_actors: impl Into<BTreeSet<&'static str>>,
        task_description: Option<String>,
        role: String,
        whitelisted_commands: Vec<String>,
        file_reader: Option<Arc<std::sync::Mutex<crate::actors::tools::file_reader::FileReader>>>,
    ) -> Self {
        let client = LLMClient::new(config.base_url.clone());

        let required_actors = required_actors.into();
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

        let system_state = if let Some(file_reader) = file_reader {
            SystemState::with_file_reader(file_reader)
        } else {
            SystemState::new()
        };

        let mut s = Self {
            tx,
            config,
            client,
            chat_history: Vec::new(),
            system_state,
            available_tools: Vec::new(),
            cancel_handle: Arc::new(Mutex::new(None)),
            pending_message: PendingMessage::new(),
            state,
            task_description,
            scope,
            parent_scope,
            live_spawned_agents_scope: BTreeSet::new(),
            whitelisted_commands,
            role,
        };

        // If we have a task and we aren't waiting on actors just submit it here
        // We handle the case where we have a task and are waiting on actors in the handle_message
        // method
        match (&s.state, &s.task_description) {
            (AgentStatus::Wait { reason }, Some(_)) => match reason {
                WaitReason::WaitingForUserInput => {
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
            self.available_tools
                .retain(|t| t.function.name != new_tool.function.name);
            // Add the new tool
            self.available_tools.push(new_tool);
        }
    }

    fn handle_tool_call_update(&mut self, update: ToolCallUpdate) {
        if let ToolCallStatus::Finished(result) = update.status {
            match &mut self.state {
                AgentStatus::Wait {
                    reason: WaitReason::WaitingForTools { tool_calls },
                } => {
                    let found = match tool_calls.get_mut(&update.call_id) {
                        Some(pending_call) => {
                            pending_call.result = Some(result);
                            true
                        }
                        None => false,
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
                                .unwrap_or_else(|e| format!("Error: {}", e));
                            self.chat_history.push(ChatMessage::tool(
                                call_id,
                                pending_call.tool_name,
                                content,
                            ));
                        }
                        self.submit_pending_message(true);
                    }
                }
                // Don't submit pending messages while WaitingForManager or WaitForSystem
                AgentStatus::Wait {
                    reason:
                        WaitReason::WaitingForManager {
                            tool_name,
                            tool_call_id,
                        },
                }
                | AgentStatus::Wait {
                    reason:
                        WaitReason::WaitForSystem {
                            tool_name,
                            tool_call_id,
                            ..
                        },
                } => {
                    if tool_call_id != &update.call_id {
                        return;
                    }

                    let content = result.unwrap_or_else(|e| format!("Error: {}", e));
                    self.chat_history.push(ChatMessage::tool(
                        update.call_id,
                        tool_name.clone().unwrap_or("system_tool".to_string()),
                        content,
                    ));
                }
                _ => (),
            }
        }
    }

    fn add_user_content(&mut self, content: String) {
        self.pending_message.set_user_content(content);
    }

    fn add_system_message(&mut self, message: String) {
        self.pending_message.add_system_message(message);
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
        self.chat_history.extend(messages);

        self.submit_llm_request();
    }

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

        let system_prompt = self.render_system_prompt();

        self.broadcast(Message::AssistantRequest(AssistantRequest {
            system: system_prompt.clone(),
            tools: self.available_tools.clone(),
            messages: self.chat_history.clone(),
        }));

        // Spawn the assist task
        let tx = self.tx.clone();
        let client = self.client.clone();
        let config = self.config.clone();
        let scope = self.scope.clone();
        let messages = self.chat_history.clone();
        let tools = self.available_tools.clone();

        let handle = tokio::spawn(async move {
            if let Err(e) = do_assist(
                tx,
                client,
                system_prompt,
                messages,
                tools,
                config,
                scope,
                processing_id,
            )
            .await
            {
                error!("Error in assist task: {:?}", e);
            }
        });

        *self.cancel_handle.lock().unwrap() = Some(handle);
    }

    fn render_system_prompt(&mut self) -> String {
        // Build tool infos for system prompt
        let tool_infos: Vec<ToolInfo> = self
            .available_tools
            .iter()
            .filter_map(|tool| {
                Some(ToolInfo {
                    name: tool.function.name.clone(),
                    description: tool.function.description.clone(),
                })
            })
            .collect();

        // Render system prompt with tools and task description
        match self.system_state.render_system_prompt_with_task_and_role(
            &self.config.system_prompt,
            &tool_infos,
            self.whitelisted_commands.clone(),
            self.task_description.clone(),
            self.role.clone(),
            self.scope,
        ) {
            Ok(rendered_prompt) => rendered_prompt,
            Err(e) => {
                error!("Failed to render system prompt: {}", e);
                "".to_string()
            }
        }
    }
}

// TODO: Need to work on some retrying / timeout logic here
async fn do_assist(
    tx: broadcast::Sender<ActorMessage>,
    client: LLMClient,
    system_prompt: String,
    messages: Vec<ChatMessage>,
    tools: Vec<Tool>,
    config: ParsedModelConfig,
    scope: Scope,
    processing_id: Uuid,
) -> Result<(), LLMError> {
    // Filter out wait tool calls as those are just noise
    let messages = crate::utils::filter_wait_tool_calls(&messages);

    // Debug log the request data
    tracing::debug!(
        "LLM Request: model={} messages_len={}, tools_len={}, messages=\n=====SYSTEM=====\n{system_prompt}\n=====MESSAGES=====\n{}",
        config.model_name,
        messages.len(),
        tools.len(),
        serde_json::to_string_pretty(&messages).unwrap()
    );

    let resp = client
        .chat(
            &config.model_name,
            &system_prompt,
            messages,
            if tools.is_empty() { None } else { Some(tools) },
        )
        .await?;

    // Debug log the full response
    tracing::debug!(
        "LLM Response for agent: {scope} | choices={:?}, usage={:?}, model={}",
        resp.choices,
        resp.usage,
        resp.model
    );

    if let Some(choice) = resp.choices.first() {
        let assistant_message = match &choice.message {
            ChatMessage::Assistant(assistant_message) => assistant_message,
            _ => {
                whatever!("LLM returned non-assistant response")
            }
        };

        // Send response
        let _ = tx.send(ActorMessage {
            scope,
            message: Message::AssistantResponse {
                id: processing_id,
                message: assistant_message.clone(),
            },
        });

        // Handle tool calls if any
        if let Some(tool_calls) = assistant_message.tool_calls.clone() {
            for tool_call in tool_calls {
                let _ = tx.send(ActorMessage {
                    scope,
                    message: Message::AssistantToolCall(tool_call),
                });
            }
        }
    } else {
        // Something strange is happening...
        if let Some(usage) = &resp.usage {
            if usage.completion_tokens > 0 {
                tracing::warn!(
                    "LLM returned no choices but consumed {} completion tokens - this may be a model-specific behavior. Response details: usage={:?}, model={}",
                    usage.completion_tokens,
                    resp.usage,
                    resp.model
                );
            } else {
                error!(
                    "No choices from assistant and no tokens consumed - Response details: choices={:?}, usage={:?}, model={}",
                    resp.choices, resp.usage, resp.model
                );
            }
        } else {
            error!(
                "No choices from assistant - Response details: choices={:?}, usage={:?}, model={}",
                resp.choices, resp.usage, resp.model
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
        self.live_spawned_agents_scope
            .iter()
            .chain([&self.scope, &self.parent_scope])
            .collect::<Vec<&Scope>>()
    }

    async fn on_stop(&mut self) {
        for scope in self.live_spawned_agents_scope.clone() {
            let _ = self.tx.send(ActorMessage {
                scope,
                message: Message::Action(Action::Exit),
            });
        }
        self.live_spawned_agents_scope.clear();
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
                                    self.add_system_message(format_manager_message(&message));
                                    match self.state.clone() {
                                        AgentStatus::Wait { reason } => match reason {
                                            WaitReason::WaitForSystem { .. } => {
                                                self.submit_pending_message(false);
                                            }
                                            WaitReason::WaitingForUserInput
                                            | WaitReason::WaitingForManager { .. } => {
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
                            if let Some(task_description) = self.task_description.clone() {
                                self.pending_message.set_user_content(task_description);
                                self.submit_pending_message(true);
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
                        )
                        | (
                            UserContext::UserTUIInput(text),
                            AgentStatus::Wait {
                                reason: WaitReason::WaitForSystem { .. },
                            },
                        ) => {
                            self.add_user_content(text);
                            self.submit_pending_message(false);
                        }
                        (UserContext::UserTUIInput(text), _) => {
                            self.add_user_content(text);
                        }
                        // Other user context is handled in the tui
                        (_, _) => (),
                    }
                }

                Message::PlanUpdated(plan) => {
                    self.system_state.update_plan(plan);
                }

                // Responses from our call to do_assist
                Message::AssistantResponse { id, message } => {
                    // State transition: Processing -> AwaitingTools or Idle/Wait based on content
                    if let AgentStatus::Processing { id: processing_id } = &self.state {
                        // This is probably an old cancelled call or something
                        if processing_id != &id {
                            return;
                        }
                        self.chat_history
                            .push(ChatMessage::Assistant(message.clone()));

                        if let Some(tool_calls) = message.tool_calls {
                            let tool_calls_map = tool_calls
                                .iter()
                                .map(|tc| {
                                    (
                                        tc.id.clone(),
                                        PendingToolCall {
                                            tool_name: tc.function.name.clone(),
                                            result: None,
                                        },
                                    )
                                })
                                .collect();
                            self.set_state(
                                AgentStatus::Wait {
                                    reason: WaitReason::WaitingForTools {
                                        tool_calls: tool_calls_map,
                                    },
                                },
                                true,
                            );
                        } else if let Some(content) = message.content {
                            if *crate::IS_HEADLESS.get().unwrap_or(&false) {
                                // This is an error by the LLM it should only ever respond with
                                // tool calls in headless mode
                                self.add_system_message("ERROR! You responded without calling a tool. Try again and this time ensure you call a tool! If in doubt, use the `wait` tool.".to_string());
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

                // Messages from our tools that update
                Message::Agent(message) => match message.message {
                    // We created a sub agent
                    AgentMessageType::AgentSpawned {
                        agent_type,
                        role,
                        task_description,
                        tool_call_id,
                    } => {
                        // Ensure we actually called the tool to spawn agents
                        if let AgentStatus::Wait {
                            reason: WaitReason::WaitingForTools { tool_calls },
                        } = &self.state
                        {
                            if !tool_calls.contains_key(&tool_call_id) {
                                return;
                            }
                            self.live_spawned_agents_scope
                                .insert(message.agent_id.clone());
                            let agent_info = crate::system_state::AgentTaskInfo::new(
                                message.agent_id.clone(),
                                agent_type,
                                role,
                                task_description,
                            );
                            self.system_state.add_agent(agent_info);
                        }
                    }

                    // One of our sub agents was removed
                    AgentMessageType::AgentRemoved => {
                        todo!();
                        // self.system_state.remove_agent(&message.agent_id);
                        // probably remove the scope here
                    }

                    // These are our own status updates and system messages broadcasted from tool calls and temporal agents
                    AgentMessageType::InterAgentMessage(inter_agent_message)
                        if message.agent_id == self.scope =>
                    {
                        match inter_agent_message {
                            InterAgentMessage::StatusUpdateRequest {
                                status,
                                tool_call_id,
                            } => {
                                if let AgentStatus::Wait {
                                    reason: WaitReason::WaitingForTools { tool_calls },
                                } = self.state.clone()
                                {
                                    if let Some(tool_call) = tool_calls.get(&tool_call_id) {
                                        // If we don't have any more live sub agents don't allow transitioning into WiatForSystem from the `wait` tool
                                        // This is a special case catch for when a Manager calls the `wait` tool and all agents are done
                                        if tool_call.tool_name == WAIT_TOOL_NAME
                                            && self.live_spawned_agents_scope.is_empty()
                                            && matches!(
                                                status,
                                                AgentStatus::Wait {
                                                    reason: WaitReason::WaitForSystem { .. }
                                                }
                                            )
                                        {
                                            self.chat_history.push(ChatMessage::tool(
                                                tool_call_id,
                                                &tool_call.tool_name,
                                                ATTEMPTED_WAIT_ERROR_MESSAGE,
                                            ));
                                            self.submit_pending_message(true);
                                        } else {
                                            self.set_state(status.clone(), true);
                                            if matches!(status, AgentStatus::Done(..)) {
                                                // When we are done we shutdown
                                                self.broadcast(Message::Action(Action::Exit));
                                            }
                                        }
                                    }
                                }
                            }
                            // TODO: Maybe move this so it doesn't require the scope to be ours?
                            // We would also need to just listen to every message from every scope
                            // where the `agent_id` is ours.
                            InterAgentMessage::InterruptAndForceWaitForManager { tool_call_id } => {
                                // If the last message is a tool call pop it off
                                // Most APIs error if a ToolCall is not followed by a ToolResponse
                                // Alternatively we could add a dummy tool response but this seems more reasonable
                                if matches!(
                                    self.chat_history.last(),
                                    Some(ChatMessage::Assistant(AssistantChatMessage {
                                        tool_calls: Some(_),
                                        ..
                                    }))
                                ) {
                                    self.chat_history.pop();
                                }

                                self.set_state(
                                    AgentStatus::Wait {
                                        reason: WaitReason::WaitingForManager {
                                            tool_name: None,
                                            tool_call_id,
                                        },
                                    },
                                    true,
                                );
                            }
                            InterAgentMessage::Message { message } => {
                                self.pending_message
                                    .add_system_message(format_general_message(&message));
                                match self.state.clone() {
                                    AgentStatus::Wait {
                                        reason: WaitReason::WaitForSystem { .. },
                                    }
                                    | AgentStatus::Wait {
                                        reason: WaitReason::WaitingForUserInput,
                                    } => {
                                        self.submit_pending_message(false);
                                    }
                                    _ => (),
                                }
                            }
                            _ => (),
                        }
                    }
                    _ => (),
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
                                    AgentStatus::Done(agent_task_result) => {
                                        self.live_spawned_agents_scope
                                            .remove(&agent_message.agent_id);

                                        let formatted_message = match &agent_task_result {
                                            Ok(response) => format_agent_response_success(
                                                &agent_message.agent_id,
                                                response.success,
                                                &response.summary,
                                            ),
                                            Err(err) => format_agent_response_failure(
                                                &agent_message.agent_id,
                                                err,
                                            ),
                                        };
                                        self.add_system_message(formatted_message);

                                        match self.state.clone() {
                                            AgentStatus::Wait {
                                                reason: WaitReason::WaitForSystem { .. },
                                            }
                                            | AgentStatus::Wait {
                                                reason: WaitReason::WaitingForUserInput,
                                            } => {
                                                self.submit_pending_message(false);
                                            }
                                            _ => (),
                                        }
                                    }
                                    _ => (),
                                }
                            }
                            InterAgentMessage::Message {
                                message: sub_agent_message,
                            } if agent_message.agent_id == self.scope => {
                                self.add_system_message(format_sub_agent_message(
                                    &sub_agent_message,
                                    &agent_message.agent_id,
                                ));
                                match self.state.clone() {
                                    AgentStatus::Wait { reason } => match reason {
                                        WaitReason::WaitForSystem { .. }
                                        | WaitReason::WaitingForUserInput
                                        | WaitReason::WaitingForManager { .. } => {
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
    use std::collections::HashMap;

    use crate::llm_client::ToolCall;

    use super::*;

    // Test Coverage for Assistant Handle Message and State Transitions
    fn create_test_assistant(
        required_actors: impl Into<BTreeSet<&'static str>>,
        task_description: Option<String>,
    ) -> Assistant {
        let parsed_config = ParsedModelConfig {
            model_name: "filler".to_string(),
            system_prompt: "Test system prompt with task: {{ task }}".to_string(),
            litellm_params: HashMap::new(),
            base_url: "http://localhost:9999".to_string(),
        };

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
            "filler role".to_string(),
            vec![],
            None,
        )
    }

    fn create_test_assistant_with_parent(
        required_actors: impl Into<BTreeSet<&'static str>>,
        task_description: Option<String>,
        parent_scope: Scope,
    ) -> Assistant {
        let parsed_config = ParsedModelConfig {
            model_name: "filler".to_string(),
            system_prompt: "Test system prompt with task: {{ task }}".to_string(),
            litellm_params: HashMap::new(),
            base_url: "http://localhost:9999".to_string(),
        };

        let (tx, _) = broadcast::channel(10);
        let scope = Scope::new();
        Assistant::new(
            parsed_config,
            tx,
            scope,
            parent_scope,
            required_actors,
            task_description,
            "filler role".to_string(),
            vec![],
            None,
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
        let assistant = create_test_assistant(["tool1", "tool2"], None);
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForActors { .. }
            }
        ));
    }

    #[test]
    fn test_initial_state_without_required_actors() {
        let assistant = create_test_assistant([], None);
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));
    }

    #[tokio::test]
    async fn test_initial_state_without_required_actors_with_task() {
        let assistant = create_test_assistant([], Some("Test task".to_string()));
        // Should immediately go to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));
    }

    // Actor Ready Message Tests

    #[tokio::test]
    async fn test_awaiting_actors_to_idle_transition() {
        let mut assistant = create_test_assistant(["tool1", "tool2"], None);

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
        let mut assistant = create_test_assistant(["tool1"], Some("Test task".to_string()));

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

        let system_message = assistant.render_system_prompt();
        assert!(system_message.contains("Test task"));
    }

    #[tokio::test]
    async fn test_actor_ready_ignored_when_not_waiting() {
        let mut assistant = create_test_assistant([], None);

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
        let mut assistant = create_test_assistant([], None);

        let tool1 = Tool {
            tool_type: "function".to_string(),
            function: crate::llm_client::ToolFunction {
                name: "test_tool1".to_string(),
                description: "Test tool 1".to_string(),
                parameters: serde_json::Value::Null,
            },
        };
        let tool2 = Tool {
            tool_type: "function".to_string(),
            function: crate::llm_client::ToolFunction {
                name: "test_tool2".to_string(),
                description: "Test tool 2".to_string(),
                parameters: serde_json::Value::Null,
            },
        };

        // Send tools available
        send_message(&mut assistant, Message::ToolsAvailable(vec![tool1.clone()])).await;
        assert_eq!(assistant.available_tools.len(), 1);
        assert_eq!(assistant.available_tools[0].function.name, "test_tool1");

        // Send more tools - should add to existing
        send_message(&mut assistant, Message::ToolsAvailable(vec![tool2.clone()])).await;
        assert_eq!(assistant.available_tools.len(), 2);

        // Send duplicate tool - should replace
        let tool1_updated = Tool {
            tool_type: "function".to_string(),
            function: crate::llm_client::ToolFunction {
                name: "test_tool1".to_string(),
                description: "Updated test tool 1".to_string(),
                parameters: serde_json::Value::Null,
            },
        };
        send_message(&mut assistant, Message::ToolsAvailable(vec![tool1_updated])).await;
        assert_eq!(assistant.available_tools.len(), 2);
        // Find the updated tool1 by name since order may vary
        let updated_tool = assistant
            .available_tools
            .iter()
            .find(|t| t.function.name == "test_tool1")
            .expect("test_tool1 should exist");
        assert_eq!(
            updated_tool.function.description,
            "Updated test tool 1".to_string()
        );
    }

    // User Input Tests

    #[tokio::test]
    async fn test_user_input_when_waiting_for_input() {
        let mut assistant = create_test_assistant([], None);

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
        let mut assistant = create_test_assistant(["tool1"], None);

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
        let mut assistant = create_test_assistant([], None);

        send_message(
            &mut assistant,
            Message::FileRead {
                path: std::path::PathBuf::from("/test/file.txt"),
                content: "File content".to_string(),
                last_modified: std::time::SystemTime::now(),
            },
        )
        .await;
    }

    #[tokio::test]
    async fn test_file_edited_message() {
        let mut assistant = create_test_assistant([], None);

        send_message(
            &mut assistant,
            Message::FileEdited {
                path: std::path::PathBuf::from("/test/file.txt"),
                content: "Updated content".to_string(),
                last_modified: std::time::SystemTime::now(),
            },
        )
        .await;
    }

    #[tokio::test]
    async fn test_plan_updated_message() {
        use crate::actors::tools::planner::TaskPlan;

        let mut assistant = create_test_assistant([], None);
        let plan = TaskPlan {
            title: "Test Plan".to_string(),
            tasks: vec![],
        };

        send_message(&mut assistant, Message::PlanUpdated(plan)).await;
    }

    // Assistant Response Tests

    #[tokio::test]
    async fn test_assistant_response_with_text() {
        let mut assistant = create_test_assistant([], None);
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
                message: AssistantChatMessage::new_with_content("Response text"),
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
        let mut assistant = create_test_assistant([], None);
        let processing_id = Uuid::new_v4();

        // Manually set to Processing state
        assistant.set_state(
            AgentStatus::Processing {
                id: processing_id.clone(),
            },
            true,
        );

        let tool_call = ToolCall {
            id: "call_123".to_string(),
            tool_type: "function".to_string(),
            function: crate::llm_client::Function {
                name: "test_tool".to_string(),
                arguments: "null".to_string(),
            },
            index: None,
        };

        // Send assistant response with tool calls
        send_message(
            &mut assistant,
            Message::AssistantResponse {
                id: processing_id,
                message: AssistantChatMessage::new_with_tools(vec![tool_call]),
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
        let mut assistant = create_test_assistant([], None);
        let current_id = Uuid::new_v4();
        let old_id = Uuid::new_v4();

        // Set to Processing with current ID
        assistant.set_state(AgentStatus::Processing { id: current_id }, true);

        // Send response with old ID
        send_message(
            &mut assistant,
            Message::AssistantResponse {
                id: old_id,
                message: AssistantChatMessage::new_with_content("Old response"),
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
        let mut assistant = create_test_assistant([], None);

        // Set up WaitingForTools state
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert(
            "call_123".to_string(),
            PendingToolCall {
                tool_name: "test_tool".to_string(),
                result: None,
            },
        );
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
        let mut assistant = create_test_assistant([], None);

        // Set up WaitingForTools state with multiple tools
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert(
            "call_1".to_string(),
            PendingToolCall {
                tool_name: "test_tool1".to_string(),
                result: None,
            },
        );
        tool_calls.insert(
            "call_2".to_string(),
            PendingToolCall {
                tool_name: "test_tool2".to_string(),
                result: None,
            },
        );
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
        let mut assistant = create_test_assistant([], None);

        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert(
            "call_123".to_string(),
            PendingToolCall {
                tool_name: "test_tool".to_string(),
                result: None,
            },
        );
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

        let mut assistant = create_test_assistant([], None);
        let spawned_agent_id = Scope::new();

        assistant.state = AgentStatus::Wait {
            reason: WaitReason::WaitingForTools {
                tool_calls: HashMap::from([(
                    "call_123".to_string(),
                    PendingToolCall {
                        tool_name: "test_tool".to_string(),
                        result: None,
                    },
                )]),
            },
        };

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
        assert_eq!(assistant.live_spawned_agents_scope.len(), 1);
        assert!(
            assistant
                .live_spawned_agents_scope
                .contains(&spawned_agent_id)
        );
    }

    // Inter-Agent Message Tests

    #[tokio::test]
    async fn test_manager_message_when_waiting() {
        let parent_scope = Scope::new();
        let mut assistant = create_test_assistant_with_parent([], None, parent_scope);

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
        let mut assistant = create_test_assistant([], None);
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.live_spawned_agents_scope.insert(sub_agent_scope);

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
        let mut assistant = create_test_assistant([], None);
        let sub_agent_id = Scope::new();
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.live_spawned_agents_scope.insert(sub_agent_scope);

        // Send sub-agent status update
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_id,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdate {
                    status: AgentStatus::Done(Ok(crate::actors::AgentTaskResultOk {
                        summary: "Task completed".to_string(),
                        success: true,
                    })),
                }),
            }),
        )
        .await;

        // Verify the message was added properly to chat history
        let last_message = assistant.chat_history.last().unwrap();
        if let ChatMessage::System { content } = last_message {
            assert!(content.contains("Task completed"));
        } else {
            panic!("Expected System message");
        }
    }

    // State Transition Tests

    #[tokio::test]
    async fn test_tool_updates_to_wait_for_duration() {
        let mut assistant = create_test_assistant([], None);

        // Set up WaitingForTools state
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert(
            "call_123".to_string(),
            PendingToolCall {
                tool_name: "test_tool".to_string(),
                result: None,
            },
        );
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
                        tool_call_id: "call_123".to_string(),
                        status: AgentStatus::Wait {
                            reason: WaitReason::WaitForSystem {
                                tool_name: None,
                                tool_call_id: "call_123".to_string(),
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
                reason: WaitReason::WaitForSystem { .. }
            }
        ));
    }

    #[tokio::test]
    async fn test_tool_updates_to_wait_for_plan_approval() {
        let mut assistant = create_test_assistant([], None);

        // Set up WaitingForTools state
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert(
            "call_123".to_string(),
            PendingToolCall {
                tool_name: "test_tool".to_string(),
                result: None,
            },
        );
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
                        tool_call_id: "call_123".to_string(),
                        status: AgentStatus::Wait {
                            reason: WaitReason::WaitingForManager {
                                tool_name: None,
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
                reason: WaitReason::WaitingForManager { .. }
            }
        ));
    }

    #[tokio::test]
    async fn test_manager_message_during_plan_approval_wait() {
        let parent_scope = Scope::new();
        let mut assistant = create_test_assistant_with_parent([], None, parent_scope);

        // Set to WaitingForPlanApproval
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForManager {
                    tool_name: None,
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
        let mut assistant = create_test_assistant_with_parent([], None, parent_scope);

        let initial_messages_count = assistant.chat_history.len();

        // Set to WaitForSystem
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitForSystem {
                    tool_name: None,
                    tool_call_id: "call_123".to_string(),
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
                    message: "Interrupt wait".to_string(),
                }),
            }),
        )
        .await;

        // Should transition to Processing
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));

        // Verify that a system response was added
        assert_eq!(assistant.chat_history.len(), initial_messages_count + 1);

        let last_message = assistant.chat_history.last().unwrap();

        if let ChatMessage::System { content } = last_message {
            assert!(content.contains("Interrupt wait"));
        } else {
            panic!("Expected System message");
        }
    }

    #[tokio::test]
    async fn test_sub_agent_message_during_system_wait() {
        let mut assistant = create_test_assistant([], None);
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.live_spawned_agents_scope.insert(sub_agent_scope);

        let initial_messages_count = assistant.chat_history.len();

        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitForSystem {
                    tool_name: None,
                    tool_call_id: "call_456".to_string(),
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

        // Verify that the message was added
        assert_eq!(assistant.chat_history.len(), initial_messages_count + 1);

        let last_message = assistant.chat_history.last().unwrap();
        assert!(matches!(last_message, ChatMessage::System { .. }));
    }

    #[tokio::test]
    async fn test_tool_completion_during_plan_approval_wait() {
        let mut assistant = create_test_assistant([], None);

        // Set to WaitingForManager
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForManager {
                    tool_name: None,
                    tool_call_id: "plan_call_123".to_string(),
                },
            },
            true,
        );

        let initial_messages_count = assistant.chat_history.len();

        // Send tool completion
        send_message(
            &mut assistant,
            Message::ToolCallUpdate(ToolCallUpdate {
                call_id: "plan_call_123".to_string(),
                status: ToolCallStatus::Finished(Ok("Plan submitted for approval".to_string())),
            }),
        )
        .await;

        // Should STILL be in WaitingForManager - does NOT transition to Processing
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForManager { .. }
            }
        ));

        // Tool response should be added to chat_history
        assert_eq!(assistant.chat_history.len(), initial_messages_count + 1);

        let last_message = assistant.chat_history.last().unwrap();
        if let ChatMessage::Tool {
            tool_call_id,
            content,
            ..
        } = last_message
        {
            assert_eq!(tool_call_id, "plan_call_123");
            assert_eq!(content, "Plan submitted for approval");
        } else {
            panic!("Expected Tool message");
        }
    }

    #[tokio::test]
    async fn test_tool_completion_during_wait_for_system() {
        let mut assistant = create_test_assistant([], None);

        // Set to WaitForDuration
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitForSystem {
                    tool_name: None,
                    tool_call_id: "wait_call_456".to_string(),
                },
            },
            true,
        );

        let initial_messages_count = assistant.chat_history.len();

        // Send tool completion
        send_message(
            &mut assistant,
            Message::ToolCallUpdate(ToolCallUpdate {
                call_id: "wait_call_456".to_string(),
                status: ToolCallStatus::Finished(Ok("Waiting...".to_string())),
            }),
        )
        .await;

        // Should stay in waiting for system
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitForSystem { .. }
            }
        ));

        // Should be added to chat_history
        assert_eq!(assistant.chat_history.len(), initial_messages_count + 1);

        let last_message = assistant.chat_history.last().unwrap();
        if let ChatMessage::Tool {
            tool_call_id,
            content,
            ..
        } = last_message
        {
            assert_eq!(tool_call_id, "wait_call_456");
            assert_eq!(content, "Waiting...");
        } else {
            panic!("Expected Tool message");
        }
    }

    // Pending Message Tests

    #[tokio::test]
    async fn test_pending_messages_accumulate() {
        let mut assistant = create_test_assistant(["tool1"], None);

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
        assert_eq!(
            assistant.pending_message.user_content,
            Some("Message 2".to_string())
        );
        assert_eq!(assistant.pending_message.system_messages.len(), 0);
    }

    #[tokio::test]
    async fn test_assistant_response_with_pending_messages() {
        let mut assistant = create_test_assistant([], None);
        let processing_id = Uuid::new_v4();

        // Add pending message
        assistant
            .pending_message
            .set_user_content("Pending message".to_string());

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
                message: AssistantChatMessage::new_with_content("Response"),
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
        let mut assistant = create_test_assistant([], None);

        // Add a tool to trigger system prompt update
        let tool = Tool {
            tool_type: "function".to_string(),
            function: crate::llm_client::ToolFunction {
                name: "test_tool".to_string(),
                description: "Test tool".to_string(),
                parameters: serde_json::Value::Null,
            },
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

        // Trigger submit
        send_message(
            &mut assistant,
            Message::UserContext(UserContext::UserTUIInput("Test".to_string())),
        )
        .await;
    }

    // Edge Case Tests

    #[tokio::test]
    async fn test_multiple_actor_ready_messages() {
        let mut assistant = create_test_assistant(["tool1"], None);

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
        let mut assistant = create_test_assistant([], None);

        // Set up WaitingForTools state
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert(
            "call_123".to_string(),
            PendingToolCall {
                tool_name: "test_tool".to_string(),
                result: None,
            },
        );
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
        let mut assistant = create_test_assistant([], None);
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
        let mut assistant = create_test_assistant([], None);

        // Set up WaitingForTools with multiple tools
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert(
            "call_1".to_string(),
            PendingToolCall {
                tool_name: "test_tool1".to_string(),
                result: None,
            },
        );
        tool_calls.insert(
            "call_2".to_string(),
            PendingToolCall {
                tool_name: "test_tool2".to_string(),
                result: None,
            },
        );
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { tool_calls },
            },
            true,
        );

        // Tool 1 requests state change to WaitForSystem
        let agent_scope = assistant.scope.clone();
        send_message(
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: agent_scope,
                message: AgentMessageType::InterAgentMessage(
                    InterAgentMessage::StatusUpdateRequest {
                        tool_call_id: "call_1".to_string(),
                        status: AgentStatus::Wait {
                            reason: WaitReason::WaitForSystem {
                                tool_name: None,
                                tool_call_id: "call_1".to_string(),
                            },
                        },
                    },
                ),
            }),
        )
        .await;

        // Should transition to WaitForSystem
        if let AgentStatus::Wait {
            reason: WaitReason::WaitForSystem { tool_call_id, .. },
        } = &assistant.state
        {
            assert_eq!(tool_call_id, "call_1");
        } else {
            panic!("Expected WaitForDuration state");
        }

        // Tool 2 can no longer update the state
        let agent_scope = assistant.scope.clone();
        send_message(
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: agent_scope,
                message: AgentMessageType::InterAgentMessage(
                    InterAgentMessage::StatusUpdateRequest {
                        tool_call_id: "call_2".to_string(),
                        status: AgentStatus::Wait {
                            reason: WaitReason::WaitingForManager {
                                tool_name: None,
                                tool_call_id: "call_2".to_string(),
                            },
                        },
                    },
                ),
            }),
        )
        .await;

        // State should now be WaitingForSystem - tools can only transition state when being
        // waited upon
        if let AgentStatus::Wait {
            reason:
                WaitReason::WaitForSystem {
                    tool_name: None,
                    tool_call_id,
                },
        } = &assistant.state
        {
            assert_eq!(tool_call_id, "call_1");
        } else {
            panic!("Expected WaitingForPlanApproval state");
        }
    }

    // Test on_stop behavior
    #[tokio::test]
    async fn test_on_stop_sends_exit_to_sub_agents() {
        let mut assistant = create_test_assistant([], None);
        let sub_agent_1 = Scope::new();
        let sub_agent_2 = Scope::new();

        // Add sub-agents
        assistant.live_spawned_agents_scope.insert(sub_agent_1);
        assistant.live_spawned_agents_scope.insert(sub_agent_2);

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
        assert!(assistant.live_spawned_agents_scope.is_empty());
    }

    // Sub-Agent Completion Tests

    #[tokio::test]
    async fn test_sub_agent_completion_during_wait_for_duration() {
        let mut assistant = create_test_assistant([], None);

        let sub_agent_scope = Scope::new();
        // assistant.spawned_agents_scope.push(sub_agent_scope);

        assistant.state = AgentStatus::Wait {
            reason: WaitReason::WaitingForTools {
                tool_calls: HashMap::from([(
                    "spawn_123".to_string(),
                    PendingToolCall {
                        tool_name: "spawn_agent".to_string(),
                        result: None,
                    },
                )]),
            },
        };

        // First spawn the sub-agent so it's in system state
        send_message(
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_scope,
                message: AgentMessageType::AgentSpawned {
                    agent_type: crate::actors::AgentType::Worker,
                    role: "test_worker".to_string(),
                    task_description: "test task".to_string(),
                    tool_call_id: "spawn_123".to_string(),
                },
            }),
        )
        .await;

        let initial_messages_count = assistant.chat_history.len();

        // Set to Wait
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitForSystem {
                    tool_name: None,
                    tool_call_id: "wait_call_123".to_string(),
                },
            },
            true,
        );

        // Send sub-agent task completion (success case)
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_scope,
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
        assert_eq!(assistant.chat_history.len(), initial_messages_count + 1);

        // Check the last message is a system message with agent response
        let last_message = assistant.chat_history.last().unwrap();
        if let ChatMessage::System { content } = last_message {
            assert!(content.contains("sub_agent_complete"));
            assert!(content.contains(&sub_agent_scope.to_string()));
            assert!(content.contains("SUCCESS"));
            assert!(content.contains("Sub-agent task completed successfully"));
        } else {
            panic!("Expected System message");
        }
    }

    #[tokio::test]
    async fn test_sub_agent_completion_during_wait_for_duration_failure() {
        let mut assistant = create_test_assistant([], None);
        let sub_agent_scope = Scope::new();

        assistant.state = AgentStatus::Wait {
            reason: WaitReason::WaitingForTools {
                tool_calls: HashMap::from([(
                    "spawn_456".to_string(),
                    PendingToolCall {
                        tool_name: "spawn_agent".to_string(),
                        result: None,
                    },
                )]),
            },
        };

        // First spawn the sub-agent so it's in system state
        send_message(
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_scope,
                message: AgentMessageType::AgentSpawned {
                    agent_type: crate::actors::AgentType::Worker,
                    role: "test_worker".to_string(),
                    task_description: "test task".to_string(),
                    tool_call_id: "spawn_456".to_string(),
                },
            }),
        )
        .await;

        let initial_messages_count = assistant.chat_history.len();

        // Set to Wait
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitForSystem {
                    tool_name: None,
                    tool_call_id: "wait_call_456".to_string(),
                },
            },
            true,
        );

        // Send sub-agent task completion (failure case)
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_scope,
                message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdate {
                    status: AgentStatus::Done(Err("Sub-agent encountered an error".to_string())),
                }),
            }),
        )
        .await;

        // Should transition to Processing due to interrupt
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));

        // Verify messages were added
        assert_eq!(assistant.chat_history.len(), initial_messages_count + 1);

        // Check the last message contains failure information
        let last_message = assistant.chat_history.last().unwrap();
        if let ChatMessage::System { content } = last_message {
            assert!(content.contains("sub_agent_complete"));
            assert!(content.contains(&sub_agent_scope.to_string()));
            assert!(content.contains("FAILURE"));
            assert!(content.contains("Sub-agent encountered an error"));
        } else {
            panic!("Expected System message");
        }
    }

    #[tokio::test]
    async fn test_sub_agent_completion_during_waiting_for_user_input() {
        let mut assistant = create_test_assistant([], None);
        let sub_agent_scope = Scope::new();
        assistant.live_spawned_agents_scope.insert(sub_agent_scope);

        // Verify in WaitingForUserInput
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForUserInput
            }
        ));

        let initial_messages_count = assistant.chat_history.len();

        // Send sub-agent task completion
        send_message_with_scope(
            &sub_agent_scope,
            &mut assistant,
            Message::Agent(crate::actors::AgentMessage {
                agent_id: sub_agent_scope,
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
        assert!(matches!(assistant.state, AgentStatus::Processing { .. }));

        // Messages should be added to chat_history
        assert_eq!(assistant.chat_history.len(), initial_messages_count + 1);

        let last_message = assistant.chat_history.last().unwrap();
        if let ChatMessage::System { content } = last_message {
            assert!(content.contains("Background task completed"));
        } else {
            panic!("Expected System message");
        }
    }

    #[tokio::test]
    async fn test_sub_agent_completion_during_processing() {
        let mut assistant = create_test_assistant([], None);
        let sub_agent_id = Scope::new();
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.live_spawned_agents_scope.insert(sub_agent_scope);

        // Set to Processing state
        let processing_id = Uuid::new_v4();
        assistant.set_state(AgentStatus::Processing { id: processing_id }, true);

        let initial_messages_count = assistant.chat_history.len();

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

        // No new messages should be added to chat_history
        assert_eq!(assistant.chat_history.len(), initial_messages_count);

        // But pending message should contain the agent response
        assert!(assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_sub_agent_completion_during_waiting_for_tools() {
        let mut assistant = create_test_assistant([], None);
        let sub_agent_id = Scope::new();
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.live_spawned_agents_scope.insert(sub_agent_scope);

        // Set up WaitingForTools state
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert(
            "call_123".to_string(),
            PendingToolCall {
                tool_name: "test_tool".to_string(),
                result: None,
            },
        );
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { tool_calls },
            },
            true,
        );

        let initial_messages_count = assistant.chat_history.len();

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

        // No new messages should be added to chat_history
        assert_eq!(assistant.chat_history.len(), initial_messages_count);

        // But pending message should contain the agent response
        assert!(assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_sub_agent_completion_during_waiting_for_plan_approval() {
        let mut assistant = create_test_assistant([], None);
        let sub_agent_id = Scope::new();
        let sub_agent_scope = Scope::new();

        // Add sub-agent to tracked scopes
        assistant.live_spawned_agents_scope.insert(sub_agent_scope);

        // Set to WaitingForPlanApproval
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForManager {
                    tool_name: None,
                    tool_call_id: "plan_123".to_string(),
                },
            },
            true,
        );

        let initial_messages_count = assistant.chat_history.len();

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
                reason: WaitReason::WaitingForManager { .. }
            }
        ));

        // No new messages should be added to chat_history
        assert_eq!(assistant.chat_history.len(), initial_messages_count);

        // But pending message should contain the agent response
        assert!(assistant.pending_message.has_content());
    }

    #[tokio::test]
    async fn test_interrupt_and_force_wait_for_manager() {
        let mut assistant = create_test_assistant_with_parent(BTreeSet::new(), None, Scope::new());
        assistant.set_state(AgentStatus::Processing { id: Uuid::new_v4() }, true);

        // Add some tool calls to chat history
        let tool_call_id = "test_tool_123".to_string();
        assistant
            .chat_history
            .push(ChatMessage::assistant_with_tools(vec![
                crate::llm_client::ToolCall {
                    id: tool_call_id.clone(),
                    tool_type: "function".to_string(),
                    function: crate::llm_client::Function {
                        name: "test_tool".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: Some(0),
                },
            ]));

        let initial_history_len = assistant.chat_history.len();

        let agent_scope = assistant.scope.clone();
        send_message(
            &mut assistant,
            Message::Agent(AgentMessage {
                agent_id: agent_scope, // Must match assistant's scope
                message: AgentMessageType::InterAgentMessage(
                    InterAgentMessage::InterruptAndForceWaitForManager {
                        tool_call_id: "manager_123".to_string(),
                    },
                ),
            }),
        )
        .await;

        // Should transition to WaitingForManager
        assert!(matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForManager { .. }
            }
        ));

        // Tool calls should be removed from chat history
        assert!(assistant.chat_history.len() == initial_history_len - 1);
    }

    #[tokio::test]
    async fn test_concurrent_tool_status_updates() {
        let mut assistant = create_test_assistant(BTreeSet::new(), None);
        let tool_call_id_1 = "tool_1".to_string();
        let tool_call_id_2 = "tool_2".to_string();

        // Create initial tool calls map
        let mut tool_calls = HashMap::new();
        tool_calls.insert(
            tool_call_id_1.clone(),
            PendingToolCall {
                tool_name: "tool1".to_string(),
                result: None,
            },
        );
        tool_calls.insert(
            tool_call_id_2.clone(),
            PendingToolCall {
                tool_name: "tool2".to_string(),
                result: None,
            },
        );

        // Set state to WaitingForTools with multiple tool calls
        assistant.set_state(
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { tool_calls },
            },
            true,
        );

        // First tool completes successfully
        send_message(
            &mut assistant,
            Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call_id_1.clone(),
                status: ToolCallStatus::Finished(Ok("Tool 1 completed".to_string())),
            }),
        )
        .await;

        // Second tool also completes
        send_message(
            &mut assistant,
            Message::ToolCallUpdate(ToolCallUpdate {
                call_id: tool_call_id_2.clone(),
                status: ToolCallStatus::Finished(Ok("Tool 2 completed".to_string())),
            }),
        )
        .await;

        // Both tools have completed, so state should change from WaitingForTools
        // to either WaitingForUserInput (if processing completes) or continue processing
        assert!(!matches!(
            assistant.state,
            AgentStatus::Wait {
                reason: WaitReason::WaitingForTools { .. }
            }
        ));
    }
}

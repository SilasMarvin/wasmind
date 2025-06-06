use genai::{
    Client,
    chat::{ChatMessage, ChatRequest, ChatRole, MessageContent, Tool},
};
use snafu::ResultExt;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    SResult,
    actors::{Actor, Message, ToolCallStatus, ToolCallUpdate, state_system::StateSystem},
    config::ParsedConfig,
    system_state::SystemState,
    template::ToolInfo,
};

use super::{ActorMessage, AgentMessageType, InterAgentMessage, TaskAwaitingManager, UserContext};

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

/// Assistant actor that handles AI interactions
pub struct Assistant {
    tx: broadcast::Sender<ActorMessage>,
    config: ParsedConfig,
    client: Client,
    chat_request: ChatRequest,
    system_state: SystemState,
    available_tools: Vec<Tool>,
    cancel_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pending_content_parts: Vec<genai::chat::ContentPart>,
    state: AssistantState,
    task_description: Option<String>,
    scope: Uuid,
    spawned_agents_scope: Vec<Uuid>,
    required_actors: Vec<&'static str>,
}

impl Assistant {
    pub fn new(
        config: ParsedConfig,
        tx: broadcast::Sender<ActorMessage>,
        scope: Uuid,
        required_actors: Vec<&'static str>,
        task_description: Option<String>,
    ) -> Self {
        let client = Client::builder()
            .with_service_target_resolver(config.model.service_target_resolver.clone())
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
            pending_content_parts: Vec::new(),
            state,
            task_description: None,
            scope,
            required_actors,
            spawned_agents_scope: vec![],
        }
    }

    #[tracing::instrument(name = "assist_request", skip(self, request), fields(tools_count = request.tools.as_ref().map_or(0, |tools| tools.len())))]
    async fn handle_assist_request(&mut self, request: ChatRequest) {
        // Cancel any existing request
        if let Some(handle) = self.cancel_handle.lock().await.take() {
            handle.abort();
        }

        self.maybe_rerender_system_prompt().await;

        // Spawn the assist task
        let tx = self.tx.clone();
        let client = self.client.clone();
        let config = self.config.clone();

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
        // Check if this is a completion
        if let ToolCallStatus::Finished(result) = &update.status {
            // Create tool response and add to chat
            let tool_response = genai::chat::ToolResponse {
                call_id: update.call_id.clone(),
                content: result.clone().unwrap_or_else(|e| format!("Error: {}", e)),
            };

            self.chat_request = self.chat_request.clone().append_message(ChatMessage {
                role: ChatRole::Tool,
                content: MessageContent::ToolResponses(vec![tool_response]),
                options: None,
            });

            // Automatically continue the conversation
            self.handle_assist_request(self.chat_request.clone()).await;
        }
    }

    #[tracing::instrument(name = "user_input", skip(self, text), fields(input_length = text.len()))]
    async fn handle_user_input(&mut self, text: String) {
        if self.pending_content_parts.is_empty() {
            // Simple text message
            self.chat_request = self
                .chat_request
                .clone()
                .append_message(ChatMessage::user(text));
        } else {
            // Multi-part message with text and other content
            let mut parts = vec![genai::chat::ContentPart::Text(text)];
            parts.append(&mut self.pending_content_parts.clone());
            self.chat_request = self
                .chat_request
                .clone()
                .append_message(ChatMessage::user(MessageContent::Parts(parts)));
            self.pending_content_parts.clear();
        }

        self.handle_assist_request(self.chat_request.clone()).await;
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
                &self.config.model.system_prompt,
                &tool_infos,
                self.config.whitelisted_commands.clone(),
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

#[tracing::instrument(name = "llm_request", skip(tx, client, chat_request, config), fields(model = %config.model.name, tools_count = chat_request.tools.as_ref().map_or(0, |tools| tools.len())))]
async fn do_assist(
    tx: broadcast::Sender<ActorMessage>,
    client: Client,
    chat_request: ChatRequest,
    config: ParsedConfig,
    scope: Uuid,
) -> SResult<()> {
    let request = chat_request;

    let resp = client
        .exec_chat(&config.model.name, request, None)
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

    async fn on_start(&mut self) {}

    async fn handle_message(&mut self, message: ActorMessage) {
        let _ = self.transition(&message.message);

        // TODO: Integrate state into here. handle_message and transition should probably be meshed
        // together

        // Messages from our tools, etc...
        if message.scope == self.scope {
            match message.message {
                Message::ToolsAvailable(tools) => self.handle_tools_available(tools).await,
                Message::ToolCallUpdate(update) => self.handle_tool_call_update(update).await,

                Message::UserContext(context) => match context {
                    #[cfg(feature = "audio")]
                    UserContext::MicrophoneTranscription(text) => {
                        self.handle_user_input(text).await
                    }
                    UserContext::UserTUIInput(text) => self.handle_user_input(text).await,
                    #[cfg(feature = "gui")]
                    UserContext::ScreenshotCaptured(result) => {
                        if let Ok(base64) = result {
                            // Add screenshot as an image content part
                            let content_part =
                                genai::chat::ContentPart::from_image_base64("image/png", base64);
                            self.pending_content_parts.push(content_part);
                        }
                        // Errors are already handled by TUI
                    }
                    #[cfg(feature = "gui")]
                    UserContext::ClipboardCaptured(_result) => {
                        // Clipboard text is sent as UserTUIInput by the TUI actor when the user hits
                        // enter so we don't need to handle it here
                    }
                },

                Message::Action(crate::actors::Action::Assist) => {
                    // Re-send current chat request
                    self.handle_assist_request(self.chat_request.clone()).await;
                }
                Message::Action(crate::actors::Action::Cancel) => {
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
                    self.maybe_rerender_system_prompt().await;
                }
                Message::PlanUpdated(plan) => {
                    info!("Updating system state with new plan: {}", plan.title);
                    self.system_state.update_plan(plan);
                }

                Message::AssistantResponse(content) => {
                    self.chat_request = self.chat_request.clone().append_message(ChatMessage {
                        role: ChatRole::Assistant,
                        content,
                        options: None,
                    });
                }

                Message::Agent(message) => match message.message {
                    AgentMessageType::AgentSpawned {
                        agent_role,
                        task_description,
                    } => {
                        let agent_info = crate::system_state::AgentTaskInfo::new(
                            message.agent_id,
                            agent_role,
                            task_description,
                        );
                        self.system_state.add_agent(agent_info);
                    }
                    AgentMessageType::AgentRemoved => {
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
                                    .update_agent_status(&message.agent_id, status);
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

impl StateSystem for Assistant {
    type State = AssistantState;

    fn current_state(&self) -> &Self::State {
        &self.state
    }

    fn transition(&mut self, message: &Message) -> Option<Self::State> {
        let new_state = match (&self.state, message) {
            (AssistantState::AwaitingActors, Message::ActorReady { actor_id }) => {
                self.required_actors = self
                    .required_actors
                    .drain(..)
                    .filter(|r_id| r_id != &actor_id.as_str())
                    .collect::<Vec<&'static str>>();

                if self.required_actors.is_empty() {
                    Some(AssistantState::Idle)
                } else {
                    None
                }
            }
            // From Idle to Processing when receiving user input or assist action
            (AssistantState::Idle, Message::UserContext(UserContext::UserTUIInput(_)))
            | (AssistantState::Idle, Message::Action(crate::actors::Action::Assist)) => {
                Some(AssistantState::Processing)
            }

            // From Processing to WaitingForTools when assistant makes tool calls
            (AssistantState::Processing, Message::AssistantResponse(content)) => {
                match content {
                    MessageContent::ToolCalls(tool_calls) => {
                        let call_ids = tool_calls.iter().map(|tc| tc.call_id.clone()).collect();
                        Some(AssistantState::AwaitingTools {
                            pending_tool_calls: call_ids,
                        })
                    }
                    // If response has no tool calls, go back to Idle
                    _ => Some(AssistantState::Idle),
                }
            }

            // From WaitingForTools back to Processing when tool finishes
            (
                AssistantState::AwaitingTools { pending_tool_calls },
                Message::ToolCallUpdate(update),
            ) => {
                if let ToolCallStatus::Finished(_) = &update.status {
                    // Only process tool updates for calls we're actually waiting for
                    if pending_tool_calls.contains(&update.call_id) {
                        let mut remaining_calls = pending_tool_calls.clone();
                        remaining_calls.retain(|id| id != &update.call_id);

                        if remaining_calls.is_empty() {
                            // All tools finished, back to processing for next LLM response
                            Some(AssistantState::Processing)
                        } else {
                            // Still waiting for more tools
                            Some(AssistantState::AwaitingTools {
                                pending_tool_calls: remaining_calls,
                            })
                        }
                    } else {
                        None // Ignore tool updates for calls we're not waiting for
                    }
                } else {
                    None // No state change for non-finished tool updates
                }
            }

            // Cancel action can move from Processing or WaitingForTools back to Idle
            (AssistantState::Processing, Message::Action(crate::actors::Action::Cancel))
            | (
                AssistantState::AwaitingTools { .. },
                Message::Action(crate::actors::Action::Cancel),
            ) => Some(AssistantState::Idle),

            // Any state can go to Error (though we don't currently track errors explicitly)
            // This would be used if we wanted to track error states explicitly
            _ => None, // No state transition
        };

        if let Some(ref new_state) = new_state {
            info!(
                "Assistant state transition: {:?} -> {:?}",
                self.state, new_state
            );
            self.state = new_state.clone();
        }

        new_state
    }
}

// TODO: Fix these tests
// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::actors::state_system::test_utils::*;
//     use genai::chat::ToolCall;
//
//     fn create_test_assistant() -> Assistant {
//         use crate::config::Config;
//         let config = Config::default().unwrap().try_into().unwrap();
//         let (tx, _) = broadcast::channel(10);
//         Assistant::new(config, tx)
//     }
//
//     #[test]
//     fn test_assistant_starts_in_idle() {
//         let assistant = create_test_assistant();
//         assert_eq!(assistant.current_state(), &AssistantState::Idle);
//     }
//
//     #[test]
//     fn test_assistant_state_transition_user_input() {
//         let mut assistant = create_test_assistant();
//         assistant.state = AssistantState::Idle; // Set to Idle state
//
//         assert_state_transition(
//             &mut assistant,
//             Message::UserTUIInput("Hello".to_string()),
//             AssistantState::Processing,
//         );
//     }
//
//     #[test]
//     fn test_assistant_state_transition_tool_calls() {
//         let mut assistant = create_test_assistant();
//         assistant.state = AssistantState::Processing;
//
//         let tool_calls = vec![ToolCall {
//             call_id: "call_123".to_string(),
//             fn_name: "test_function".to_string(),
//             fn_arguments: serde_json::json!({}),
//         }];
//
//         assert_state_transition(
//             &mut assistant,
//             Message::AssistantResponse(MessageContent::ToolCalls(tool_calls)),
//             AssistantState::WaitingForTools {
//                 pending_tool_calls: vec!["call_123".to_string()],
//             },
//         );
//     }
//
//     #[test]
//     fn test_assistant_state_transition_tool_finished() {
//         let mut assistant = create_test_assistant();
//         assistant.state = AssistantState::WaitingForTools {
//             pending_tool_calls: vec!["call_123".to_string()],
//         };
//
//         let update = ToolCallUpdate {
//             call_id: "call_123".to_string(),
//             status: ToolCallStatus::Finished(Ok("Success".to_string())),
//         };
//
//         assert_state_transition(
//             &mut assistant,
//             Message::ToolCallUpdate(update),
//             AssistantState::Processing,
//         );
//     }
//
//     #[test]
//     fn test_assistant_no_transition_wrong_message() {
//         let mut assistant = create_test_assistant();
//         assistant.state = AssistantState::Idle;
//
//         // Random message that shouldn't cause transition from Idle
//         assert_no_state_transition(
//             &mut assistant,
//             Message::Action(crate::actors::Action::CaptureWindow),
//         );
//     }
//
//     #[test]
//     fn test_user_input_while_processing_dropped() {
//         let mut assistant = create_test_assistant();
//         assistant.state = AssistantState::Processing;
//
//         // User input should be dropped while processing
//         assert_no_state_transition(
//             &mut assistant,
//             Message::UserTUIInput("Another request".to_string()),
//         );
//     }
//
//     #[test]
//     fn test_user_input_while_waiting_for_tools_dropped() {
//         let mut assistant = create_test_assistant();
//         assistant.state = AssistantState::WaitingForTools {
//             pending_tool_calls: vec!["call_123".to_string()],
//         };
//
//         // User input should be dropped while waiting for tools
//         assert_no_state_transition(
//             &mut assistant,
//             Message::UserTUIInput("Impatient user input".to_string()),
//         );
//     }
//
//     #[test]
//     fn test_tool_update_while_not_waiting_dropped() {
//         let mut assistant = create_test_assistant();
//         assistant.state = AssistantState::Idle;
//
//         let update = ToolCallUpdate {
//             call_id: "unexpected_call".to_string(),
//             status: ToolCallStatus::Finished(Ok("Success".to_string())),
//         };
//
//         // Tool update should be dropped when not waiting for tools
//         assert_no_state_transition(&mut assistant, Message::ToolCallUpdate(update));
//     }
//
//     #[test]
//     fn test_tool_update_while_processing_dropped() {
//         let mut assistant = create_test_assistant();
//         assistant.state = AssistantState::Processing;
//
//         let update = ToolCallUpdate {
//             call_id: "unexpected_call".to_string(),
//             status: ToolCallStatus::Finished(Ok("Success".to_string())),
//         };
//
//         // Tool update should be dropped while processing (before tools are called)
//         assert_no_state_transition(&mut assistant, Message::ToolCallUpdate(update));
//     }
//
//     #[test]
//     fn test_wrong_tool_call_id_ignored() {
//         let mut assistant = create_test_assistant();
//         assistant.state = AssistantState::WaitingForTools {
//             pending_tool_calls: vec!["call_123".to_string()],
//         };
//
//         let update = ToolCallUpdate {
//             call_id: "call_456".to_string(), // Different call ID
//             status: ToolCallStatus::Finished(Ok("Success".to_string())),
//         };
//
//         // Tool update with wrong call ID should be ignored
//         assert_no_state_transition(&mut assistant, Message::ToolCallUpdate(update));
//     }
//
//     #[test]
//     fn test_multiple_user_inputs_while_processing() {
//         let mut assistant = create_test_assistant();
//         assistant.state = AssistantState::Processing;
//
//         // Multiple user inputs should all be dropped
//         assert_no_state_transition(
//             &mut assistant,
//             Message::UserTUIInput("First request".to_string()),
//         );
//
//         assert_no_state_transition(
//             &mut assistant,
//             Message::MicrophoneTranscription("Second request".to_string()),
//         );
//
//         assert_no_state_transition(
//             &mut assistant,
//             Message::Action(crate::actors::Action::Assist),
//         );
//
//         // Should still be in Processing state
//         assert_eq!(assistant.current_state(), &AssistantState::Processing);
//     }
//
//     #[test]
//     fn test_partial_tool_completion_maintains_waiting_state() {
//         let mut assistant = create_test_assistant();
//         assistant.state = AssistantState::WaitingForTools {
//             pending_tool_calls: vec!["call_123".to_string(), "call_456".to_string()],
//         };
//
//         let update = ToolCallUpdate {
//             call_id: "call_123".to_string(),
//             status: ToolCallStatus::Finished(Ok("Success".to_string())),
//         };
//
//         // Should transition to still waiting but with one less pending call
//         assert_state_transition(
//             &mut assistant,
//             Message::ToolCallUpdate(update),
//             AssistantState::WaitingForTools {
//                 pending_tool_calls: vec!["call_456".to_string()],
//             },
//         );
//     }
// }

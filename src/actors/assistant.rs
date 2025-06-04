use genai::{
    Client,
    chat::{ChatMessage, ChatRequest, ChatRole, MessageContent, Tool},
};
use snafu::ResultExt;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tracing::{error, info};

use crate::{
    SResult,
    actors::{Actor, Message, ToolCallStatus, ToolCallUpdate, state_system::StateSystem},
    config::ParsedConfig,
    system_state::SystemState,
    template::ToolInfo,
};

/// States that the Assistant actor can be in
#[derive(Debug, Clone, PartialEq)]
pub enum AssistantState {
    /// Ready to accept requests, has tools available
    Idle,
    /// Actively processing a user request (making LLM call)
    Processing,
    /// Waiting for tool execution results
    WaitingForTools { pending_tool_calls: Vec<String> },
    /// Encountered an error during processing
    Error { message: String },
}

/// Assistant actor that handles AI interactions
pub struct Assistant {
    tx: broadcast::Sender<Message>,
    config: ParsedConfig,
    client: Client,
    chat_request: ChatRequest,
    system_state: SystemState,
    available_tools: Vec<Tool>,
    cancel_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pending_content_parts: Vec<genai::chat::ContentPart>,
    state: AssistantState,
}

impl Assistant {
    async fn handle_assist_request(&mut self, request: ChatRequest) {
        info!("Assistant received assist request");

        // Cancel any existing request
        if let Some(handle) = self.cancel_handle.lock().await.take() {
            handle.abort();
        }

        // Spawn the assist task
        let tx = self.tx.clone();
        let client = self.client.clone();
        let config = self.config.clone();

        error!(
            "DOING CHAT REQUEST WITH:\n{:?}",
            serde_json::to_string_pretty(&request).unwrap()
        );

        let handle = tokio::spawn(async move {
            if let Err(e) = do_assist(tx, client, request, config).await {
                error!("Error in assist task: {:?}", e);
            }
        });

        *self.cancel_handle.lock().await = Some(handle);

        info!("Done assisting");
    }

    async fn handle_tools_available(&mut self, new_tools: Vec<Tool>) {
        info!("Assistant received {} new tools", new_tools.len());

        // Add new tools to existing tools
        for new_tool in new_tools {
            // Remove any existing tool with the same name
            self.available_tools.retain(|t| t.name != new_tool.name);
            // Add the new tool
            self.available_tools.push(new_tool);
        }

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

        // Render system prompt with tools
        match self.system_state.render_system_prompt(
            &self.config.model.system_prompt,
            &tool_infos,
            self.config.whitelisted_commands.clone(),
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
                error!("Failed to render system prompt: {}", e);
            }
        }
    }

    async fn handle_tool_call_update(&mut self, update: ToolCallUpdate) {
        // Check if this is a completion
        if let ToolCallStatus::Finished(result) = &update.status {
            info!("Tool call {} finished", update.call_id);

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

    async fn handle_microphone_transcription(&mut self, text: String) {
        info!("Assistant received microphone transcription");

        // Add user message to chat
        self.chat_request = self
            .chat_request
            .clone()
            .append_message(ChatMessage::user(text));

        // Send assist request
        self.handle_assist_request(self.chat_request.clone()).await;
    }

    async fn handle_user_input(&mut self, text: String) {
        info!("Assistant received user input");

        info!("Got pending parts");

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

        info!("About to send assist request");

        self.handle_assist_request(self.chat_request.clone()).await;
    }

    async fn maybe_rerender_system_prompt(&mut self) {
        if self.system_state.is_modified() {
            info!("System state modified, re-rendering system prompt");

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

            // Render system prompt with tools
            match self.system_state.render_system_prompt(
                &self.config.model.system_prompt,
                &tool_infos,
                self.config.whitelisted_commands.clone(),
            ) {
                Ok(rendered_prompt) => {
                    self.chat_request = self
                        .chat_request
                        .clone()
                        .with_system(&rendered_prompt)
                        .with_tools(self.available_tools.clone());
                    self.system_state.reset_modified();
                    info!("System prompt re-rendered successfully");
                }
                Err(e) => {
                    error!("Failed to re-render system prompt: {}", e);
                }
            }
        }
    }
}

async fn do_assist(
    tx: broadcast::Sender<Message>,
    client: Client,
    chat_request: ChatRequest,
    config: ParsedConfig,
) -> SResult<()> {
    let request = chat_request;

    info!("Executing chat request");
    let resp = client
        .exec_chat(&config.model.name, request, None)
        .await
        .context(crate::GenaiSnafu)?;

    info!("Got resp: {:?}", resp);

    if let Some(message_content) = resp.content {
        info!("Got message content: {:?}", message_content);

        // Note: We don't update chat_request here since it's owned by this function
        // The Assistant struct will handle updating its own chat_request when needed

        // Send response
        let _ = tx.send(Message::AssistantResponse(message_content.clone()));

        // Handle tool calls if any
        if let MessageContent::ToolCalls(tool_calls) = message_content {
            for tool_call in tool_calls {
                tx.send(Message::AssistantToolCall(tool_call.clone()))
                    .expect("Error sending tool call");
            }
        }
    } else {
        error!("No message content from assistant: {:?}", resp);
    }

    Ok(())
}

#[async_trait::async_trait]
impl Actor for Assistant {
    const ACTOR_ID: &'static str = "assistant";

    fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self {
        let client = Client::builder()
            .with_service_target_resolver(config.model.service_target_resolver.clone())
            .build();

        Self {
            tx,
            config,
            client,
            chat_request: ChatRequest::default(),
            system_state: SystemState::new(),
            available_tools: Vec::new(),
            cancel_handle: Arc::new(Mutex::new(None)),
            pending_content_parts: Vec::new(),
            state: AssistantState::Idle,
        }
    }

    fn get_rx(&self) -> broadcast::Receiver<Message> {
        self.tx.subscribe()
    }

    fn get_tx(&self) -> broadcast::Sender<Message> {
        self.tx.clone()
    }

    async fn on_start(&mut self) {
        info!("Assistant actor starting...");
    }

    async fn handle_message(&mut self, message: Message) {
        match message {
            Message::ToolsAvailable(tools) => self.handle_tools_available(tools).await,
            Message::ToolCallUpdate(update) => self.handle_tool_call_update(update).await,
            #[cfg(feature = "audio")]
            Message::MicrophoneTranscription(text) => {
                self.handle_microphone_transcription(text).await
            }
            Message::UserTUIInput(text) => self.handle_user_input(text).await,
            Message::Action(crate::actors::Action::Assist) => {
                // Re-send current chat request
                self.handle_assist_request(self.chat_request.clone()).await;
            }
            Message::Action(crate::actors::Action::Cancel) => {
                // Cancel current request
                if let Some(handle) = self.cancel_handle.lock().await.take() {
                    handle.abort();
                    info!("Cancelled assist request");
                }
            }
            #[cfg(feature = "gui")]
            Message::ScreenshotCaptured(result) => {
                if let Ok(base64) = result {
                    // Add screenshot as an image content part
                    let content_part =
                        genai::chat::ContentPart::from_image_base64("image/png", base64);
                    self.pending_content_parts.push(content_part);

                    // Send user input to trigger assistant
                    let _ = self
                        .tx
                        .send(Message::UserTUIInput("[Screenshot captured]".to_string()));
                }
                // Errors are already handled by TUI
            }
            #[cfg(feature = "gui")]
            Message::ClipboardCaptured(_result) => {
                // Clipboard text is sent as UserTUIInput by the TUI actor
                // so we don't need to handle it here
            }
            Message::FileRead {
                path,
                content,
                last_modified,
            } => {
                info!("Updating system state with read file: {}", path.display());
                self.system_state.update_file(path, content, last_modified);
                self.maybe_rerender_system_prompt().await;
            }
            Message::FileEdited {
                path,
                content,
                last_modified,
            } => {
                info!("Updating system state with edited file: {}", path.display());
                self.system_state.update_file(path, content, last_modified);
                self.maybe_rerender_system_prompt().await;
            }
            Message::PlanUpdated(plan) => {
                info!("Updating system state with new plan: {}", plan.title);
                self.system_state.update_plan(plan);
                self.maybe_rerender_system_prompt().await;
            }
            Message::AssistantResponse(content) => {
                info!("Assistant received response message, adding to chat history");
                self.chat_request = self.chat_request.clone().append_message(ChatMessage {
                    role: ChatRole::Assistant,
                    content,
                    options: None,
                });
            }
            Message::AgentSpawned {
                agent_id,
                agent_role,
                task_id,
                task_description,
            } => {
                info!("Agent spawned: {} ({})", agent_role, agent_id.0);
                let agent_info = crate::system_state::AgentTaskInfo::new(
                    agent_id,
                    agent_role,
                    task_id,
                    task_description,
                );
                self.system_state.add_agent(agent_info);
                self.maybe_rerender_system_prompt().await;
            }
            Message::AgentStatusUpdate { agent_id, status } => {
                info!("Agent status update: {:?}", status);
                self.system_state.update_agent_status(&agent_id, status);
                self.maybe_rerender_system_prompt().await;
            }
            Message::AgentRemoved { agent_id } => {
                info!("Agent removed: {}", agent_id.0);
                self.system_state.remove_agent(&agent_id);
                self.maybe_rerender_system_prompt().await;
            }
            _ => {}
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
            // From Idle to Processing when receiving user input or assist action
            (AssistantState::Idle, Message::UserTUIInput(_))
            | (AssistantState::Idle, Message::Action(crate::actors::Action::Assist)) => {
                Some(AssistantState::Processing)
            }
            #[cfg(feature = "audio")]
            (AssistantState::Idle, Message::MicrophoneTranscription(_)) => {
                Some(AssistantState::Processing)
            }

            // From Processing to WaitingForTools when assistant makes tool calls
            (AssistantState::Processing, Message::AssistantResponse(content)) => {
                match content {
                    MessageContent::ToolCalls(tool_calls) => {
                        let call_ids = tool_calls.iter().map(|tc| tc.call_id.clone()).collect();
                        Some(AssistantState::WaitingForTools {
                            pending_tool_calls: call_ids,
                        })
                    }
                    // If response has no tool calls, go back to Idle
                    _ => Some(AssistantState::Idle),
                }
            }

            // From WaitingForTools back to Processing when tool finishes
            (
                AssistantState::WaitingForTools { pending_tool_calls },
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
                            Some(AssistantState::WaitingForTools {
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
                AssistantState::WaitingForTools { .. },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::state_system::test_utils::*;
    use genai::chat::ToolCall;

    fn create_test_assistant() -> Assistant {
        use crate::config::Config;
        let config = Config::default().unwrap().try_into().unwrap();
        let (tx, _) = broadcast::channel(10);
        Assistant::new(config, tx)
    }

    #[test]
    fn test_assistant_starts_in_idle() {
        let assistant = create_test_assistant();
        assert_eq!(assistant.current_state(), &AssistantState::Idle);
    }

    #[test]
    fn test_assistant_state_transition_user_input() {
        let mut assistant = create_test_assistant();
        assistant.state = AssistantState::Idle; // Set to Idle state

        assert_state_transition(
            &mut assistant,
            Message::UserTUIInput("Hello".to_string()),
            AssistantState::Processing,
        );
    }

    #[test]
    fn test_assistant_state_transition_tool_calls() {
        let mut assistant = create_test_assistant();
        assistant.state = AssistantState::Processing;

        let tool_calls = vec![ToolCall {
            call_id: "call_123".to_string(),
            fn_name: "test_function".to_string(),
            fn_arguments: serde_json::json!({}),
        }];

        assert_state_transition(
            &mut assistant,
            Message::AssistantResponse(MessageContent::ToolCalls(tool_calls)),
            AssistantState::WaitingForTools {
                pending_tool_calls: vec!["call_123".to_string()],
            },
        );
    }

    #[test]
    fn test_assistant_state_transition_tool_finished() {
        let mut assistant = create_test_assistant();
        assistant.state = AssistantState::WaitingForTools {
            pending_tool_calls: vec!["call_123".to_string()],
        };

        let update = ToolCallUpdate {
            call_id: "call_123".to_string(),
            status: ToolCallStatus::Finished(Ok("Success".to_string())),
        };

        assert_state_transition(
            &mut assistant,
            Message::ToolCallUpdate(update),
            AssistantState::Processing,
        );
    }

    #[test]
    fn test_assistant_no_transition_wrong_message() {
        let mut assistant = create_test_assistant();
        assistant.state = AssistantState::Idle;

        // Random message that shouldn't cause transition from Idle
        assert_no_state_transition(
            &mut assistant,
            Message::Action(crate::actors::Action::CaptureWindow),
        );
    }

    #[test]
    fn test_user_input_while_processing_dropped() {
        let mut assistant = create_test_assistant();
        assistant.state = AssistantState::Processing;

        // User input should be dropped while processing
        assert_no_state_transition(
            &mut assistant,
            Message::UserTUIInput("Another request".to_string()),
        );
    }

    #[test]
    fn test_user_input_while_waiting_for_tools_dropped() {
        let mut assistant = create_test_assistant();
        assistant.state = AssistantState::WaitingForTools {
            pending_tool_calls: vec!["call_123".to_string()],
        };

        // User input should be dropped while waiting for tools
        assert_no_state_transition(
            &mut assistant,
            Message::UserTUIInput("Impatient user input".to_string()),
        );
    }

    #[test]
    fn test_tool_update_while_not_waiting_dropped() {
        let mut assistant = create_test_assistant();
        assistant.state = AssistantState::Idle;

        let update = ToolCallUpdate {
            call_id: "unexpected_call".to_string(),
            status: ToolCallStatus::Finished(Ok("Success".to_string())),
        };

        // Tool update should be dropped when not waiting for tools
        assert_no_state_transition(&mut assistant, Message::ToolCallUpdate(update));
    }

    #[test]
    fn test_tool_update_while_processing_dropped() {
        let mut assistant = create_test_assistant();
        assistant.state = AssistantState::Processing;

        let update = ToolCallUpdate {
            call_id: "unexpected_call".to_string(),
            status: ToolCallStatus::Finished(Ok("Success".to_string())),
        };

        // Tool update should be dropped while processing (before tools are called)
        assert_no_state_transition(&mut assistant, Message::ToolCallUpdate(update));
    }

    #[test]
    fn test_wrong_tool_call_id_ignored() {
        let mut assistant = create_test_assistant();
        assistant.state = AssistantState::WaitingForTools {
            pending_tool_calls: vec!["call_123".to_string()],
        };

        let update = ToolCallUpdate {
            call_id: "call_456".to_string(), // Different call ID
            status: ToolCallStatus::Finished(Ok("Success".to_string())),
        };

        // Tool update with wrong call ID should be ignored
        assert_no_state_transition(&mut assistant, Message::ToolCallUpdate(update));
    }

    #[test]
    fn test_multiple_user_inputs_while_processing() {
        let mut assistant = create_test_assistant();
        assistant.state = AssistantState::Processing;

        // Multiple user inputs should all be dropped
        assert_no_state_transition(
            &mut assistant,
            Message::UserTUIInput("First request".to_string()),
        );

        assert_no_state_transition(
            &mut assistant,
            Message::MicrophoneTranscription("Second request".to_string()),
        );

        assert_no_state_transition(
            &mut assistant,
            Message::Action(crate::actors::Action::Assist),
        );

        // Should still be in Processing state
        assert_eq!(assistant.current_state(), &AssistantState::Processing);
    }

    #[test]
    fn test_partial_tool_completion_maintains_waiting_state() {
        let mut assistant = create_test_assistant();
        assistant.state = AssistantState::WaitingForTools {
            pending_tool_calls: vec!["call_123".to_string(), "call_456".to_string()],
        };

        let update = ToolCallUpdate {
            call_id: "call_123".to_string(),
            status: ToolCallStatus::Finished(Ok("Success".to_string())),
        };

        // Should transition to still waiting but with one less pending call
        assert_state_transition(
            &mut assistant,
            Message::ToolCallUpdate(update),
            AssistantState::WaitingForTools {
                pending_tool_calls: vec!["call_456".to_string()],
            },
        );
    }
}

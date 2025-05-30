use genai::{
    Client,
    chat::{ChatMessage, ChatRequest, ChatRole, MessageContent, Tool},
};
use snafu::ResultExt;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tracing::{error, info};

use crate::{
    GenaiSnafu, SResult,
    actors::{Actor, Message, ToolCallStatus, ToolCallUpdate},
    config::ParsedConfig,
    system_state::SystemState,
    template::ToolInfo,
};

/// Assistant actor that handles AI interactions
pub struct Assistant {
    tx: broadcast::Sender<Message>,
    config: ParsedConfig,
    client: Client,
    chat_request: Arc<Mutex<ChatRequest>>,
    system_state: Arc<Mutex<SystemState>>,
    available_tools: Arc<Mutex<Vec<Tool>>>,
    cancel_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pending_content_parts: Arc<Mutex<Vec<genai::chat::ContentPart>>>,
}

impl Assistant {
    async fn handle_assist_request(&mut self, request: ChatRequest) {
        info!("Assistant received assist request");

        // Cancel any existing request
        if let Some(handle) = self.cancel_handle.lock().await.take() {
            handle.abort();
        }

        // Update the chat request
        *self.chat_request.lock().await = request.clone();

        // Spawn the assist task
        let tx = self.tx.clone();
        let client = self.client.clone();
        let config = self.config.clone();
        let chat_request = self.chat_request.clone();

        let handle = tokio::spawn(async move {
            if let Err(e) = do_assist(tx, client, chat_request, config).await {
                error!("Error in assist task: {}", e);
            }
        });

        *self.cancel_handle.lock().await = Some(handle);

        info!("Done assisting");
    }

    async fn handle_tools_available(&mut self, tools: Vec<Tool>) {
        info!("Assistant received {} tools", tools.len());

        // Update available tools
        *self.available_tools.lock().await = tools.clone();

        // Build tool infos for system prompt
        let tool_infos: Vec<ToolInfo> = tools
            .iter()
            .filter_map(|tool| {
                tool.description.as_ref().map(|desc| ToolInfo {
                    name: tool.name.clone(),
                    description: desc.clone(),
                })
            })
            .collect();

        // Render system prompt with tools
        let mut system_state = self.system_state.lock().await;
        match system_state.render_system_prompt(
            &self.config.model.system_prompt,
            &tool_infos,
            self.config.whitelisted_commands.clone(),
        ) {
            Ok(rendered_prompt) => {
                let mut chat_request = self.chat_request.lock().await;
                *chat_request = chat_request
                    .clone()
                    .with_system(&rendered_prompt)
                    .with_tools(tools);
                system_state.reset_modified();
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

            let chat_request = {
                let mut chat_request = self.chat_request.lock().await;
                *chat_request = chat_request.clone().append_message(ChatMessage {
                    role: ChatRole::Tool,
                    content: MessageContent::ToolResponses(vec![tool_response]),
                    options: None,
                });

                chat_request.clone()
            };

            // Automatically continue the conversation
            self.handle_assist_request(chat_request).await;
        }
    }

    async fn handle_microphone_transcription(&mut self, text: String) {
        info!("Assistant received microphone transcription");

        // Add user message to chat
        let chat_request = {
            let mut chat_request = self.chat_request.lock().await;
            *chat_request = chat_request.clone().append_message(ChatMessage::user(text));
            chat_request.clone()
        };

        // Send assist request
        self.handle_assist_request(chat_request).await;
    }

    async fn handle_user_input(&mut self, text: String) {
        info!("Assistant received user input");

        // Check if we have pending content parts
        let chat_request = {
            let mut pending_parts = self.pending_content_parts.lock().await;

            info!("Got pending parts");

            // Add user message to chat
            let mut chat_request = self.chat_request.lock().await;

            info!("Locking chat_request");

            if pending_parts.is_empty() {
                // Simple text message
                *chat_request = chat_request.clone().append_message(ChatMessage::user(text));
            } else {
                // Multi-part message with text and other content
                let mut parts = vec![genai::chat::ContentPart::Text(text)];
                parts.append(&mut pending_parts.clone());
                *chat_request = chat_request
                    .clone()
                    .append_message(ChatMessage::user(MessageContent::Parts(parts)));
                pending_parts.clear();
            }
            chat_request.clone()
        };

        info!("About to send assist request");

        self.handle_assist_request(chat_request).await;
    }
}

async fn do_assist(
    tx: broadcast::Sender<Message>,
    client: Client,
    chat_request: Arc<Mutex<ChatRequest>>,
    config: ParsedConfig,
) -> SResult<()> {
    let request = chat_request.lock().await.clone();

    info!("Executing chat request");
    let resp = client
        .exec_chat(&config.model.name, request, None)
        .await
        .context(GenaiSnafu)?;

    info!("Got resp: {:?}", resp);

    if let Some(message_content) = resp.content {
        info!("Got message content: {:?}", message_content);

        // Add assistant response to chat history
        let mut chat_request = chat_request.lock().await;
        *chat_request = chat_request
            .clone()
            .append_message(ChatMessage::assistant(message_content.clone()));

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
    fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self {
        let client = Client::builder()
            .with_service_target_resolver(config.model.service_target_resolver.clone())
            .build();

        Self {
            tx,
            config,
            client,
            chat_request: Arc::new(Mutex::new(ChatRequest::default())),
            system_state: Arc::new(Mutex::new(SystemState::new())),
            available_tools: Arc::new(Mutex::new(Vec::new())),
            cancel_handle: Arc::new(Mutex::new(None)),
            pending_content_parts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn get_rx(&self) -> broadcast::Receiver<Message> {
        self.tx.subscribe()
    }

    async fn handle_message(&mut self, message: Message) {
        info!("RECIVE IN ASSISTANT: {:?}", message);

        match message {
            Message::ToolsAvailable(tools) => self.handle_tools_available(tools).await,
            Message::ToolCallUpdate(update) => self.handle_tool_call_update(update).await,
            Message::MicrophoneTranscription(text) => {
                self.handle_microphone_transcription(text).await
            }
            Message::UserTUIInput(text) => self.handle_user_input(text).await,
            Message::UserAction(crate::actors::UserAction::Assist) => {
                // Re-send current chat request
                let request = self.chat_request.lock().await.clone();
                self.handle_assist_request(request).await;
            }
            Message::UserAction(crate::actors::UserAction::CancelAssist) => {
                // Cancel current request
                if let Some(handle) = self.cancel_handle.lock().await.take() {
                    handle.abort();
                    info!("Cancelled assist request");
                }
            }
            Message::ScreenshotCaptured(result) => {
                if let Ok(base64) = result {
                    // Add screenshot as an image content part
                    let content_part =
                        genai::chat::ContentPart::from_image_base64("image/png", base64);
                    self.pending_content_parts.lock().await.push(content_part);

                    // Send user input to trigger assistant
                    let _ = self
                        .tx
                        .send(Message::UserTUIInput("[Screenshot captured]".to_string()));
                }
                // Errors are already handled by TUI
            }
            Message::ClipboardCaptured(_result) => {
                // Clipboard text is sent as UserTUIInput by the TUI actor
                // so we don't need to handle it here
            }
            _ => {}
        }
    }
}

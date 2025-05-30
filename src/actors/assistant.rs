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
    chat_request: ChatRequest,
    system_state: SystemState,
    available_tools: Vec<Tool>,
    cancel_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pending_content_parts: Vec<genai::chat::ContentPart>,
}

impl Assistant {
    async fn handle_assist_request(&mut self, request: ChatRequest) {
        info!("Assistant received assist request");

        // Cancel any existing request
        if let Some(handle) = self.cancel_handle.lock().await.take() {
            handle.abort();
        }

        // Update the chat request
        self.chat_request = request.clone();

        // Spawn the assist task
        let tx = self.tx.clone();
        let client = self.client.clone();
        let config = self.config.clone();
        let chat_request = request;

        let handle = tokio::spawn(async move {
            if let Err(e) = do_assist(tx, client, chat_request, config).await {
                error!("Error in assist task: {}", e);
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
        let tool_infos: Vec<ToolInfo> = self.available_tools
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
                self.chat_request = self.chat_request
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
        self.chat_request = self.chat_request.clone().append_message(ChatMessage::user(text));

        // Send assist request
        self.handle_assist_request(self.chat_request.clone()).await;
    }

    async fn handle_user_input(&mut self, text: String) {
        info!("Assistant received user input");

        info!("Got pending parts");

        if self.pending_content_parts.is_empty() {
            // Simple text message
            self.chat_request = self.chat_request.clone().append_message(ChatMessage::user(text));
        } else {
            // Multi-part message with text and other content
            let mut parts = vec![genai::chat::ContentPart::Text(text)];
            parts.append(&mut self.pending_content_parts.clone());
            self.chat_request = self.chat_request
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
            let tool_infos: Vec<ToolInfo> = self.available_tools
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
                    self.chat_request = self.chat_request
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
        .context(GenaiSnafu)?;

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
            Message::ClipboardCaptured(_result) => {
                // Clipboard text is sent as UserTUIInput by the TUI actor
                // so we don't need to handle it here
            }
            Message::FileRead { path, content, last_modified } => {
                info!("Updating system state with read file: {}", path.display());
                self.system_state.update_file(path, content, last_modified);
                self.maybe_rerender_system_prompt().await;
            }
            Message::FileEdited { path, content, last_modified } => {
                info!("Updating system state with edited file: {}", path.display());
                self.system_state.update_file(path, content, last_modified);
                self.maybe_rerender_system_prompt().await;
            }
            Message::PlanUpdated(plan) => {
                info!("Updating system state with new plan: {}", plan.title);
                self.system_state.update_plan(plan);
                self.maybe_rerender_system_prompt().await;
            }
            _ => {}
        }
    }
}

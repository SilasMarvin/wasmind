use crate::actors::{Message, ToolCallStatus, ToolCallUpdate};
use crate::config::ParsedConfig;
use genai::chat::{Tool, ToolCall};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompleteInput {
    /// Summary of what was accomplished
    summary: String,
    /// Whether the task was successful
    success: bool,
}

/// Tool for agents to explicitly signal task completion
pub struct Complete {
    #[allow(dead_code)]
    config: ParsedConfig,
    tx: broadcast::Sender<Message>,
}

impl Complete {
    const TOOL_NAME: &'static str = "complete";

    pub fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self {
        Self { config, tx }
    }

    pub fn get_tool_schema() -> Tool {
        Tool {
            name: Self::TOOL_NAME.to_string(),
            description: Some("Call this tool when you have completed your assigned task. Use this to provide a summary of what was accomplished and signal that the task is finished.".to_string()),
            schema: Some(json!({
                "type": "object",
                "properties": {
                    "summary": {
                        "type": "string",
                        "description": "A brief summary of what was accomplished"
                    },
                    "success": {
                        "type": "boolean",
                        "description": "Whether the task was completed successfully (true) or failed (false)"
                    }
                },
                "required": ["summary", "success"]
            })),
        }
    }

    pub async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        if tool_call.fn_name != Self::TOOL_NAME {
            return;
        }

        // Parse input
        let input: CompleteInput = match serde_json::from_value(tool_call.fn_arguments) {
            Ok(input) => input,
            Err(e) => {
                let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!("Invalid input: {}", e))),
                }));
                return;
            }
        };

        // Send completion signal
        let completion_message = if input.success {
            format!("✅ Task completed successfully: {}", input.summary)
        } else {
            format!("❌ Task failed: {}", input.summary)
        };

        // Send tool call completion
        let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(completion_message.clone())),
        }));

        // Send task completion signal
        let _ = self.tx.send(Message::TaskCompleted {
            summary: input.summary,
            success: input.success,
        });
    }
}

#[async_trait::async_trait]
impl crate::actors::Actor for Complete {
    const ACTOR_ID: &'static str = "complete";

    fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self {
        Self::new(config, tx)
    }

    fn get_rx(&self) -> broadcast::Receiver<Message> {
        self.tx.subscribe()
    }

    fn get_tx(&self) -> broadcast::Sender<Message> {
        self.tx.clone()
    }

    async fn handle_message(&mut self, message: Message) {
        match message {
            Message::AssistantToolCall(tool_call) => {
                self.handle_tool_call(tool_call).await;
            }
            _ => {}
        }
    }

    async fn on_start(&mut self) {
        // Broadcast tool availability
        let _ = self
            .tx
            .send(Message::ToolsAvailable(vec![Self::get_tool_schema()]));
    }
}


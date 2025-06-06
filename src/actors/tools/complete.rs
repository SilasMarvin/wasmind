use crate::actors::{Actor, ActorMessage, Message, ToolCallStatus, ToolCallUpdate};
use crate::config::ParsedConfig;
use genai::chat::{Tool, ToolCall};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

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
    tx: broadcast::Sender<ActorMessage>,
    scope: Uuid,
}

impl Complete {
    const TOOL_NAME: &'static str = "complete";

    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Uuid) -> Self {
        Self { config, tx, scope }
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

        tracing::debug!(
            name = "complete_tool_call",
            call_id = %tool_call.call_id,
            "Agent called complete tool to signal task completion"
        );

        // Parse input
        let input: CompleteInput = match serde_json::from_value(tool_call.fn_arguments) {
            Ok(input) => input,
            Err(e) => {
                tracing::debug!(
                    name = "complete_tool_parse_error",
                    error = %e,
                    "Failed to parse complete tool arguments"
                );
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!("Invalid input: {}", e))),
                }));
                return;
            }
        };

        tracing::debug!(
            name = "task_completion_signal",
            summary = %input.summary,
            success = input.success,
            "Agent signaling task completion via complete tool"
        );

        // Send completion signal
        let completion_message = if input.success {
            format!("✅ Task completed successfully: {}", input.summary)
        } else {
            format!("❌ Task failed: {}", input.summary)
        };

        // Send tool call completion
        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(completion_message.clone())),
        }));

        // Send task completion signal
        let _ = self.broadcast(Message::TaskCompleted {
            summary: input.summary,
            success: input.success,
        });
    }
}

#[async_trait::async_trait]
impl Actor for Complete {
    const ACTOR_ID: &'static str = "complete";

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    fn get_scope(&self) -> &Uuid {
        &self.scope
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        match message.message {
            Message::AssistantToolCall(tool_call) => {
                self.handle_tool_call(tool_call).await;
            }
            _ => {}
        }
    }

    async fn on_start(&mut self) {
        // Broadcast tool availability
        let _ = self.broadcast(Message::ToolsAvailable(vec![Self::get_tool_schema()]));
    }
}

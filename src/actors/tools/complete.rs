use crate::actors::{
    Actor, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, AgentTaskResultOk,
    InterAgentMessage, Message, ToolCallStatus, ToolCallType, ToolCallUpdate,
};
use crate::config::ParsedConfig;
use crate::scope::Scope;
use genai::chat::{Tool, ToolCall};
use serde_json::json;
use tokio::sync::broadcast;

/// Tool for agents to explicitly signal task completion
pub struct Complete {
    #[allow(dead_code)]
    config: ParsedConfig,
    tx: broadcast::Sender<ActorMessage>,
    scope: Scope,
}

impl Complete {
    const TOOL_NAME: &'static str = "complete";

    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
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

        // Broadcast received
        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id.clone(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::Complete,
                friendly_command_display: "Calling complete".to_string(),
            },
        }));

        // Parse input
        let agent_task_result: AgentTaskResultOk =
            match serde_json::from_value(tool_call.fn_arguments.clone()) {
                Ok(input) => input,
                Err(e) => {
                    let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                        call_id: tool_call.call_id,
                        status: ToolCallStatus::Finished(Err(format!("Invalid input: {}", e))),
                    }));
                    return;
                }
            };

        // Send agent status update first to stop LLM processing
        let _ = self.broadcast(Message::Agent(AgentMessage {
            agent_id: self.get_scope().clone(),
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdateRequest {
                tool_call_id: tool_call.call_id.clone(),
                status: AgentStatus::Done(Ok(agent_task_result.clone())),
            }),
        }));

        // Send tool call completion after Done status
        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(format!(
                "Task completed{}",
                if agent_task_result.success {
                    " successfully"
                } else {
                    " with failures"
                }
            ))),
        }));
    }
}

#[async_trait::async_trait]
impl Actor for Complete {
    const ACTOR_ID: &'static str = "complete";

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    fn get_scope(&self) -> &Scope {
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
        let _ = self.broadcast(Message::ToolsAvailable(vec![Self::get_tool_schema()]));
    }
}

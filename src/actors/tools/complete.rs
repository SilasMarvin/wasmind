use crate::actors::{
    Action, Actor, ActorMessage, AgentMessage, AgentMessageType, AgentTaskResultOk,
    AgentTaskStatus, InterAgentMessage, Message, ToolCallStatus, ToolCallUpdate,
};
use crate::config::ParsedConfig;
use genai::chat::{Tool, ToolCall};
use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

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

        // Send agent status update
        let _ = self.broadcast(Message::Agent(AgentMessage {
            agent_id: self.get_scope().clone(),
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                status: AgentTaskStatus::Done(Ok(agent_task_result.clone())),
            }),
        }));

        // When the task is completed we shut down this agent
        let _ = self.broadcast(Message::Action(Action::Exit));

        // Send tool call completion with concise message
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
        let _ = self.broadcast(Message::ToolsAvailable(vec![Self::get_tool_schema()]));
    }
}

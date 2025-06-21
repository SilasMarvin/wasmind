use crate::actors::{
    Actor, ActorMessage, AgentMessage, AgentMessageType, InterAgentMessage, Message,
    TaskAwaitingManager, ToolCallStatus, ToolCallUpdate,
};
use crate::config::ParsedConfig;
use crate::scope::Scope;
use genai::chat::{Tool, ToolCall};
use serde::Deserialize;
use tokio::sync::broadcast;
use tracing::info;

pub const REQUEST_INFO_TOOL_NAME: &str = "request_information";
pub const REQUEST_INFO_TOOL_DESCRIPTION: &str = "Request additional information from your manager when you need clarification or more details to complete your task";
pub const REQUEST_INFO_TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "request": {
            "type": "string",
            "description": "Describe what information you need from your manager"
        }
    },
    "required": ["request"]
}"#;

#[derive(Debug, Deserialize)]
struct RequestInformationInput {
    request: String,
}

/// Format information request tool result message
pub fn format_information_request_sent(request: &str) -> String {
    format!("Information request sent to manager: {}", request)
}

/// RequestInformation tool actor for agents to request information from their manager
pub struct RequestInformation {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)] // TODO: Use for timeout configuration
    config: ParsedConfig,
    scope: Scope,
}

impl RequestInformation {
    pub const ACTOR_ID: &'static str = "request_information";

    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self { config, tx, scope }
    }

    pub fn get_tool_schema() -> Tool {
        Tool {
            name: REQUEST_INFO_TOOL_NAME.to_string(),
            description: Some(REQUEST_INFO_TOOL_DESCRIPTION.to_string()),
            schema: Some(
                serde_json::from_str(REQUEST_INFO_TOOL_INPUT_SCHEMA)
                    .expect("Invalid REQUEST_INFO_TOOL_INPUT_SCHEMA"),
            ),
        }
    }

    async fn handle_request_information(&mut self, tool_call: ToolCall) {
        let input: RequestInformationInput = match serde_json::from_value(tool_call.fn_arguments) {
            Ok(input) => input,
            Err(e) => {
                let error_msg = format!("Invalid request_information arguments: {}", e);
                let _ = self.tx.send(ActorMessage {
                    scope: self.scope,
                    message: Message::ToolCallUpdate(ToolCallUpdate {
                        call_id: tool_call.call_id,
                        status: ToolCallStatus::Finished(Err(error_msg)),
                    }),
                });
                return;
            }
        };

        let _ = self.broadcast(Message::Agent(AgentMessage {
            agent_id: self.get_scope().clone(),
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                status: crate::actors::AgentStatus::AwaitingManager(
                    TaskAwaitingManager::AwaitingMoreInformation(input.request.clone()),
                ),
            }),
        }));

        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id,
            status: ToolCallStatus::Finished(Ok(format_information_request_sent(&input.request))),
        }));
    }

    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        match tool_call.fn_name.as_str() {
            REQUEST_INFO_TOOL_NAME => self.handle_request_information(tool_call).await,
            _ => {}
        }
    }
}

#[async_trait::async_trait]
impl Actor for RequestInformation {
    const ACTOR_ID: &'static str = "request_information";

    fn get_scope(&self) -> &Scope {
        &self.scope
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        match message.message {
            Message::AssistantToolCall(tool_call) if message.scope == self.scope => {
                self.handle_tool_call(tool_call).await;
            }
            _ => {}
        }
    }

    async fn on_start(&mut self) {
        let _ = self.tx.send(ActorMessage {
            scope: self.scope,
            message: Message::ToolsAvailable(vec![Self::get_tool_schema()]),
        });
    }
}

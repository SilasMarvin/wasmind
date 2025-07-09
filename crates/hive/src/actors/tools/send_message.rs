use crate::actors::{
    ActorContext, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, InterAgentMessage,
    Message, ToolCallResult, WaitReason,
};
use crate::config::ParsedConfig;
use crate::llm_client::ToolCall;
use crate::scope::Scope;
use serde::Deserialize;
use tokio::sync::broadcast;

use super::Tool;

const TOOL_NAME: &str = "send_message";
const TOOL_DESCRIPTION: &str = "Send a message to a subordinate agent";
const TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "agent_id": {
            "type": "string",
            "description": "The ID of the agent to send the message to"
        },
        "message": {
            "type": "string",
            "description": "The message to send"
        },
        "wait": {
            "type": "boolean",
            "description": "If `true` pause and wait for a response else continue performing actions (default `false`)"
        }
    },
    "required": ["agent_id", "message"]
}"#;

#[derive(Debug, Deserialize)]
pub struct SendMessageInput {
    agent_id: String,
    message: String,
    wait: Option<bool>,
}

/// Format send message success message
pub fn format_send_message_success(agent_id: &str) -> String {
    format!("Message sent to agent {agent_id} - please allow at least 5 minutes for a response.")
}

/// SendMessage tool actor for managers to send messages to subordinate agents
#[derive(hive_macros::ActorContext)]
pub struct SendMessage {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)] // TODO: Use for timeout configuration
    config: ParsedConfig,
    scope: Scope,
}

impl SendMessage {
    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self { config, tx, scope }
    }
}

#[async_trait::async_trait]
impl Tool for SendMessage {
    const TOOL_NAME: &str = TOOL_NAME;
    const TOOL_DESCRIPTION: &str = TOOL_DESCRIPTION;
    const TOOL_INPUT_SCHEMA: &str = TOOL_INPUT_SCHEMA;

    type Params = SendMessageInput;

    async fn execute_tool_call(&mut self, tool_call: ToolCall, params: Self::Params) {
        // Parse the agent ID
        let agent_scope = match params.agent_id.parse::<uuid::Uuid>() {
            Ok(uuid) => Scope::from_uuid(uuid),
            Err(e) => {
                let error_msg = format!("Invalid agent ID format: {}", e);
                self.broadcast_finished(&tool_call.id, ToolCallResult::Err(error_msg), None);
                return;
            }
        };

        // Send the message
        self.broadcast(Message::Agent(AgentMessage {
            agent_id: agent_scope,
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::Message {
                message: params.message,
            }),
        }));

        // Maybe broadcast a request to wait
        if params.wait.unwrap_or_default() {
            self.broadcast(Message::Agent(AgentMessage {
                agent_id: self.scope.clone(),
                message: AgentMessageType::InterAgentMessage(
                    InterAgentMessage::StatusUpdateRequest {
                        tool_call_id: tool_call.id.clone(),
                        status: AgentStatus::Wait {
                            reason: WaitReason::WaitForSystem {
                                tool_name: Some(TOOL_NAME.to_string()),
                                tool_call_id: tool_call.id.clone(),
                            },
                        },
                    },
                ),
            }));
        }

        // Send tool success response
        self.broadcast_finished(
            &tool_call.id,
            ToolCallResult::Ok(format_send_message_success(&params.agent_id)),
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_message_deserialize_params_success() {
        let json_input = r#"{
            "agent_id": "12345678-1234-1234-1234-123456789012",
            "message": "Hello, agent!",
            "wait": false
        }"#;

        let result: Result<SendMessageInput, _> = serde_json::from_str(json_input);
        assert!(result.is_ok());

        let params = result.unwrap();
        assert_eq!(params.agent_id, "12345678-1234-1234-1234-123456789012");
        assert_eq!(params.message, "Hello, agent!");
        assert_eq!(params.wait, Some(false));
    }

    #[test]
    fn test_send_message_deserialize_params_failure() {
        let json_input = r#"{
            "message": "Hello, agent!",
            "wait": false
        }"#;

        let result: Result<SendMessageInput, _> = serde_json::from_str(json_input);
        assert!(result.is_err());
    }
}

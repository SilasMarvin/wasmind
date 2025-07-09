use crate::actors::{
    ActorContext, ActorMessage, AgentMessage, AgentMessageType, AgentStatus,
    InterAgentMessage, Message, ToolCallResult, ToolCallStatus, ToolCallUpdate, WaitReason,
};
use crate::config::ParsedConfig;
use crate::llm_client::ToolCall;
use crate::scope::Scope;
use serde::Deserialize;
use tokio::sync::broadcast;

use super::Tool;

pub const SEND_MANAGER_MESSAGE_SUCCESS_TOOL_RESPONSE: &'static str =
    "Message sent to manager. Expect a response in 300 seconds.";

const TOOL_NAME: &str = "send_manager_message";
const TOOL_DESCRIPTION: &str = "Send a message to your manager";
const TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "message": {
            "type": "string",
            "description": "The message to send to your manager"
        }
    },
    "wait": {
        "type": "boolean",
        "description": "If `true` pause and wait for a response from your manager else continue performing actions (default `false`)"
    },
    "required": ["message"]
}"#;

#[derive(Debug, Deserialize)]
pub struct SendManagerMessageInput {
    message: String,
    wait: Option<bool>,
}

/// SendManagerMessage tool actor for agents to send messages to their manager
#[derive(hive_macros::ActorContext)]
pub struct SendManagerMessage {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)] // TODO: Use for timeout configuration
    config: ParsedConfig,
    scope: Scope,
    parent_scope: Scope,
}

impl SendManagerMessage {
    pub fn new(
        config: ParsedConfig,
        tx: broadcast::Sender<ActorMessage>,
        scope: Scope,
        parent_scope: Scope,
    ) -> Self {
        Self {
            config,
            tx,
            scope,
            parent_scope,
        }
    }

}

#[async_trait::async_trait]
impl Tool for SendManagerMessage {
    const TOOL_NAME: &str = TOOL_NAME;
    const TOOL_DESCRIPTION: &str = TOOL_DESCRIPTION;
    const TOOL_INPUT_SCHEMA: &str = TOOL_INPUT_SCHEMA;

    type Params = SendManagerMessageInput;

    async fn execute_tool_call(&mut self, tool_call: ToolCall, params: Self::Params) {
        // Send the SubAgentMessage to the parent manager
        self.broadcast(Message::Agent(AgentMessage {
            agent_id: self.parent_scope,
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
                            reason: WaitReason::WaitingForManager {
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
            ToolCallResult::Ok(SEND_MANAGER_MESSAGE_SUCCESS_TOOL_RESPONSE.to_string()),
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_manager_message_deserialize_params_success() {
        let json_input = r#"{
            "message": "Hello, manager!",
            "wait": true
        }"#;
        
        let result: Result<SendManagerMessageInput, _> = serde_json::from_str(json_input);
        assert!(result.is_ok());
        
        let params = result.unwrap();
        assert_eq!(params.message, "Hello, manager!");
        assert_eq!(params.wait, Some(true));
    }

    #[test]
    fn test_send_manager_message_deserialize_params_failure() {
        let json_input = r#"{
            "wait": true
        }"#;
        
        let result: Result<SendManagerMessageInput, _> = serde_json::from_str(json_input);
        assert!(result.is_err());
    }
}

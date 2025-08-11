use crate::actors::{
    ActorContext, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, AgentTaskResultOk,
    InterAgentMessage, Message, ToolCallResult, ToolDisplayInfo,
};
use crate::config::ParsedConfig;
use crate::llm_client::ToolCall;
use crate::scope::Scope;
use tokio::sync::broadcast;

use super::Tool;

/// Tool for agents to explicitly signal task completion
#[derive(wasmind_macros::ActorContext)]
pub struct CompleteTool {
    #[allow(dead_code)]
    config: ParsedConfig,
    tx: broadcast::Sender<ActorMessage>,
    scope: Scope,
}

impl CompleteTool {
    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self { config, tx, scope }
    }
}

#[async_trait::async_trait]
impl Tool for CompleteTool {
    const TOOL_NAME: &str = "complete";
    const TOOL_DESCRIPTION: &str = "Call this tool when you have completed your assigned task. Use this to provide a summary of what was accomplished and signal that the task is finished.";
    const TOOL_INPUT_SCHEMA: &str = r#"{
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
    }"#;

    type Params = AgentTaskResultOk;

    async fn execute_tool_call(&mut self, tool_call: ToolCall, params: Self::Params) {
        // Send agent status update first to stop LLM processing
        self.broadcast(Message::Agent(AgentMessage {
            agent_id: self.get_scope().clone(),
            message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdateRequest {
                tool_call_id: tool_call.id.clone(),
                status: AgentStatus::Done(Ok(params.clone())),
            }),
        }));

        // Send tool call completion after Done status
        let result_message = format!(
            "Task completed{}",
            if params.success {
                " successfully"
            } else {
                " with failures"
            }
        );

        let tui_display = ToolDisplayInfo {
            collapsed: format!(
                "{}: {}",
                if params.success {
                    format!("{} Completed", crate::actors::tui::icons::CHECK_MARK)
                } else {
                    format!("{} Failed", crate::actors::tui::icons::X)
                },
                params.summary
            ),
            expanded: Some(params.summary.clone()),
        };

        self.broadcast_finished(
            &tool_call.id,
            ToolCallResult::Ok(result_message),
            Some(tui_display),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    fn create_test_complete() -> CompleteTool {
        let (tx, _) = broadcast::channel(100);
        let config = crate::config::Config::new(true)
            .unwrap()
            .try_into()
            .unwrap();
        let scope = Scope::new();
        CompleteTool::new(config, tx, scope)
    }

    #[test]
    fn test_complete_deserialize_params_success() {
        let complete = create_test_complete();
        let json_input = r#"{
            "summary": "Task completed successfully",
            "success": true
        }"#;

        let result = complete.deserialize_params(json_input);
        assert!(result.is_ok());

        let params = result.unwrap();
        assert_eq!(params.summary, "Task completed successfully");
        assert_eq!(params.success, true);
    }

    #[test]
    fn test_complete_deserialize_params_failure() {
        let complete = create_test_complete();
        let json_input = r#"{"summary": "Task completed successfully"}"#;

        let result = complete.deserialize_params(json_input);
        assert!(result.is_err());
    }
}

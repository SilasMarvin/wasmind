use hive::config::{Config, ParsedConfig};
use hive::scope::Scope;
use serde_json::{Value, json};
use std::sync::Once;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static INIT: Once = Once::new();

pub fn init_test_logger() {
    INIT.call_once(|| {
        // Initialize logger with a path in /workspace since that's where tests run in Docker
        hive::init_logger_with_path("/workspace/log.txt");
    });
}

pub fn create_test_config_with_mock_endpoint(mock_endpoint: String) -> ParsedConfig {
    let mut config = Config::new(true).unwrap();

    // Use gpt-4o for all models
    config.hive.main_manager_model.model_name = Some("gpt-4o".to_string());
    config.hive.sub_manager_model.model_name = Some("gpt-4o".to_string());
    config.hive.worker_model.model_name = Some("gpt-4o".to_string());

    // Set mock endpoint via litellm_params
    // let endpoint_with_v1 = format!("{}/v1/", mock_endpoint);
    // config.hive.main_manager_model.litellm_params.insert(
    //     "api_base".to_string(),
    //     toml::Value::String(endpoint_with_v1.clone()),
    // );
    // config.hive.sub_manager_model.litellm_params.insert(
    //     "api_base".to_string(),
    //     toml::Value::String(endpoint_with_v1.clone()),
    // );
    // config.hive.worker_model.litellm_params.insert(
    //     "api_base".to_string(),
    //     toml::Value::String(endpoint_with_v1),
    // );

    // Override system prompt templates to just use agent ID for easier testing
    config.hive.main_manager_model.system_prompt = "{{id}}".to_string();
    config.hive.sub_manager_model.system_prompt = "{{id}}".to_string();
    config.hive.worker_model.system_prompt = "{{id}}".to_string();

    config.try_into().unwrap()
}

/// Utilities for creating mockito mocks with proper body matching and responses

/// Represents a tool call in the LLM response
#[derive(Debug, Clone)]
pub struct MockToolCall {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

impl MockToolCall {
    fn new(call_id: impl Into<String>, tool_name: impl Into<String>, arguments: Value) -> Self {
        Self {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            arguments,
        }
    }
}

/// Fluent API for creating sequential LLM mock conversations
#[derive(Debug)]
pub struct MockSequence<'a> {
    mock_server: &'a MockServer,
    agent_scope: Scope,
    user_message: String,
    conversation_history: Vec<ConversationStep>,
}

#[derive(Debug, Clone)]
enum ConversationStep {
    ToolCall {
        response_id: String,
        tool_call: MockToolCall,
        delay: Option<Duration>,
    },
    ToolResult {
        call_id: String,
        result: String,
    },
    SystemMessage {
        content: String,
    },
    ContentResponse {
        response_id: String,
        content: String,
        delay: Option<Duration>,
    },
}

// Helper struct for exact message matching in wiremock
struct ExactMessageMatcher {
    expected_messages: Value,
}

impl wiremock::Match for ExactMessageMatcher {
    fn matches(&self, request: &wiremock::Request) -> bool {
        if let Ok(body_str) = std::str::from_utf8(&request.body) {
            if let Ok(body_json) = serde_json::from_str::<Value>(body_str) {
                if let (Some(actual_msgs), Some(expected_msgs)) = (
                    body_json.get("messages"),
                    self.expected_messages.get("messages"),
                ) {
                    if let (Some(actual_array), Some(expected_array)) =
                        (actual_msgs.as_array(), expected_msgs.as_array())
                    {
                        // Must have exact same number of messages
                        if actual_array.len() == expected_array.len() {
                            // Check if the actual messages contain all expected messages
                            for expected_msg in expected_array {
                                if !actual_array
                                    .iter()
                                    .any(|actual_msg| messages_match(expected_msg, actual_msg))
                                {
                                    return false;
                                }
                            }
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

// Helper function for matching individual messages
fn messages_match(expected: &Value, actual: &Value) -> bool {
    // Match role
    if expected.get("role") != actual.get("role") {
        return false;
    }

    // Match content if present
    if let Some(expected_content) = expected.get("content") {
        if actual.get("content") != Some(expected_content) {
            return false;
        }
    }

    // Match tool_calls if present
    if let Some(expected_calls) = expected.get("tool_calls") {
        if actual.get("tool_calls") != Some(expected_calls) {
            return false;
        }
    }

    // Match tool_call_id if present
    if let Some(expected_id) = expected.get("tool_call_id") {
        if actual.get("tool_call_id") != Some(expected_id) {
            return false;
        }
    }

    true
}

impl<'a> MockSequence<'a> {
    /// Start a new mock sequence for an agent's conversation
    pub fn new(
        mock_server: &'a MockServer,
        agent_scope: Scope,
        user_message: impl Into<String>,
    ) -> Self {
        Self {
            mock_server,
            agent_scope,
            user_message: user_message.into(),
            conversation_history: Vec::new(),
        }
    }

    /// Add a tool call response that the LLM should make
    pub fn responds_with_tool_call(
        mut self,
        response_id: impl Into<String>,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        args: Value,
    ) -> Self {
        self.responds_with_tool_call_delay(response_id, call_id, tool_name, args, None)
    }

    /// Add a tool call response with optional delay
    pub fn responds_with_tool_call_delay(
        mut self,
        response_id: impl Into<String>,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        args: Value,
        delay: Option<Duration>,
    ) -> Self {
        let tool_call = MockToolCall::new(call_id, tool_name, args);
        self.conversation_history.push(ConversationStep::ToolCall {
            response_id: response_id.into(),
            tool_call,
            delay,
        });
        self
    }

    /// Add a tool result that should be returned to the LLM
    pub fn then_expects_tool_result(
        mut self,
        call_id: impl Into<String>,
        result: impl Into<String>,
    ) -> Self {
        self.conversation_history
            .push(ConversationStep::ToolResult {
                call_id: call_id.into(),
                result: result.into(),
            });
        self
    }

    /// Add a system message to the conversation
    pub fn then_system_message(mut self, content: impl Into<String>) -> Self {
        self.conversation_history
            .push(ConversationStep::SystemMessage {
                content: content.into(),
            });
        self
    }

    /// Add a content response that the LLM should make
    pub fn responds_with_content(
        mut self,
        response_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        self.responds_with_content_delay(response_id, content, None)
    }

    /// Add a content response with optional delay
    pub fn responds_with_content_delay(
        mut self,
        response_id: impl Into<String>,
        content: impl Into<String>,
        delay: Option<Duration>,
    ) -> Self {
        self.conversation_history
            .push(ConversationStep::ContentResponse {
                response_id: response_id.into(),
                content: content.into(),
                delay,
            });
        self
    }

    /// Convenience method for read_file tool call
    pub fn responds_with_read_file(
        self,
        response_id: impl Into<String>,
        call_id: impl Into<String>,
        file_path: &str,
    ) -> Self {
        self.responds_with_tool_call(
            response_id,
            call_id,
            "read_file",
            json!({
                "path": file_path
            }),
        )
    }

    /// Convenience method for edit_file tool call
    pub fn responds_with_edit_file(
        self,
        response_id: impl Into<String>,
        call_id: impl Into<String>,
        file_path: &str,
        action: &str,
        replacement_text: &str,
    ) -> Self {
        self.responds_with_tool_call(
            response_id,
            call_id,
            "edit_file",
            json!({
                "path": file_path,
                "action": action,
                "replacement_text": replacement_text
            }),
        )
    }

    /// Convenience method for complete tool call
    pub fn responds_with_complete(
        self,
        response_id: impl Into<String>,
        call_id: impl Into<String>,
        summary: &str,
        success: bool,
    ) -> Self {
        self.responds_with_tool_call(
            response_id,
            call_id,
            "complete",
            json!({
                "summary": summary,
                "success": success
            }),
        )
    }

    /// Convenience method for approve_plan tool call
    pub fn responds_with_approve_plan(
        self,
        response_id: impl Into<String>,
        call_id: impl Into<String>,
        agent_id: &str,
    ) -> Self {
        self.responds_with_approve_plan_delay(response_id, call_id, agent_id, None)
    }

    /// Convenience method for approve_plan tool call with delay
    pub fn responds_with_approve_plan_delay(
        self,
        response_id: impl Into<String>,
        call_id: impl Into<String>,
        agent_id: &str,
        delay: Option<Duration>,
    ) -> Self {
        self.responds_with_tool_call_delay(
            response_id,
            call_id,
            "approve_plan",
            json!({
                "agent_id": agent_id
            }),
            delay,
        )
    }

    /// Convenience method for reject_plan tool call
    pub fn responds_with_reject_plan(
        self,
        response_id: impl Into<String>,
        call_id: impl Into<String>,
        agent_id: &str,
        reason: &str,
    ) -> Self {
        self.responds_with_tool_call(
            response_id,
            call_id,
            "reject_plan",
            json!({
                "agent_id": agent_id,
                "reason": reason
            }),
        )
    }

    /// Convenience method for spawn_agents tool call
    pub fn responds_with_spawn_agents(
        self,
        response_id: impl Into<String>,
        call_id: impl Into<String>,
        agents: Vec<Value>,
        wait: bool,
    ) -> Self {
        self.responds_with_spawn_agents_delay(response_id, call_id, agents, wait, None)
    }

    /// Convenience method for spawn_agents tool call with delay
    pub fn responds_with_spawn_agents_delay(
        self,
        response_id: impl Into<String>,
        call_id: impl Into<String>,
        agents: Vec<Value>,
        wait: bool,
        delay: Option<Duration>,
    ) -> Self {
        self.responds_with_tool_call_delay(
            response_id,
            call_id,
            "spawn_agents",
            json!({
                "agents_to_spawn": agents,
                "wait": wait
            }),
            delay,
        )
    }

    /// Build and mount all the mocks for this conversation sequence
    pub async fn build(self) {
        let agent_id = self.agent_scope.to_string();

        // Find ToolCall and ContentResponse steps and mount mocks for them
        for (i, step) in self.conversation_history.iter().enumerate() {
            match step {
                ConversationStep::ToolCall {
                    response_id,
                    tool_call,
                    delay,
                } => {
                    // Build the expected messages for this tool call step
                    let expected_messages = self.build_expected_messages_for_step(&agent_id, i);
                    let response_body =
                        create_tool_call_response(response_id, vec![tool_call.clone()]);

                    let mut response_template =
                        ResponseTemplate::new(200).set_body_json(response_body);
                    if let Some(delay) = delay {
                        response_template = response_template.set_delay(*delay);
                    }

                    Mock::given(method("POST"))
                        .and(path("/v1/chat/completions"))
                        .and(ExactMessageMatcher {
                            expected_messages: expected_messages.clone(),
                        })
                        .respond_with(response_template)
                        .up_to_n_times(1)
                        .mount(self.mock_server)
                        .await;
                }
                ConversationStep::ContentResponse {
                    response_id,
                    content,
                    delay,
                } => {
                    // Build the expected messages for this content response step
                    let expected_messages = self.build_expected_messages_for_step(&agent_id, i);
                    let response_body = create_content_response(response_id, content);

                    let mut response_template =
                        ResponseTemplate::new(200).set_body_json(response_body);
                    if let Some(delay) = delay {
                        response_template = response_template.set_delay(*delay);
                    }

                    Mock::given(method("POST"))
                        .and(path("/v1/chat/completions"))
                        .and(ExactMessageMatcher {
                            expected_messages: expected_messages.clone(),
                        })
                        .respond_with(response_template)
                        .up_to_n_times(1)
                        .mount(self.mock_server)
                        .await;
                }
                _ => {} // Skip other step types
            }
        }
    }

    /// Build the expected messages array for a specific step in the conversation
    fn build_expected_messages_for_step(&self, agent_id: &str, step_index: usize) -> Value {
        let mut messages = vec![
            json!({
                "role": "system",
                "content": agent_id
            }),
            json!({
                "role": "user",
                "content": [
                    {
                        "text": self.user_message,
                        "type": "text"
                    }
                ]
            }),
        ];

        // Add conversation history up to (but not including) this step
        // This ensures we see the conversation state just before making the current tool call
        for (i, step) in self.conversation_history.iter().enumerate() {
            if i >= step_index {
                break;
            }

            match step {
                ConversationStep::ToolCall { tool_call, .. } => {
                    // Add assistant message with tool call
                    messages.push(json!({
                        "role": "assistant",
                        "tool_calls": [{
                            "id": tool_call.call_id,
                            "type": "function",
                            "function": {
                                "name": tool_call.tool_name,
                                "arguments": tool_call.arguments.to_string()
                            }
                        }]
                    }));
                }
                ConversationStep::ToolResult { call_id, result } => {
                    // Add tool result message
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": result
                    }));
                }
                ConversationStep::SystemMessage { content } => {
                    // Add system message
                    messages.push(json!({
                        "role": "system",
                        "content": content
                    }));
                }
                ConversationStep::ContentResponse { content, .. } => {
                    // Add assistant content response to conversation
                    messages.push(json!({
                        "role": "assistant",
                        "content": content
                    }));
                }
            }
        }

        json!({ "messages": messages })
    }
}

/// Create a new mock sequence (fluent API entry point)
pub fn create_mock_sequence(
    mock_server: &MockServer,
    agent_scope: Scope,
    user_message: impl Into<String>,
) -> MockSequence {
    MockSequence::new(mock_server, agent_scope, user_message)
}

// Internal helper functions used by MockSequence

/// Create a chat completion response with tool calls
fn create_tool_call_response(response_id: &str, tool_calls: Vec<MockToolCall>) -> Value {
    let tool_calls_json: Vec<Value> = tool_calls
        .into_iter()
        .map(|tc| {
            json!({
                "id": tc.call_id,
                "type": "function",
                "function": {
                    "name": tc.tool_name,
                    "arguments": tc.arguments.to_string()
                }
            })
        })
        .collect();

    json!({
        "id": response_id,
        "object": "chat.completion",
        "created": 1677652288,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": tool_calls_json
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    })
}

/// Creates a mock LLM response with content (no tool calls)
fn create_content_response(response_id: &str, content: &str) -> Value {
    json!({
        "id": response_id,
        "object": "chat.completion",
        "created": 1677652288,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content,
                "tool_calls": null
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    })
}

/// Helper to create a single agent for spawn_agents
pub fn create_agent_spec(role: &str, task: &str, agent_type: &str) -> Value {
    json!({
        "agent_role": role,
        "task_description": task,
        "agent_type": agent_type
    })
}

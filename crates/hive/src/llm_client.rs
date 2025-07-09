use serde::{Deserialize, Serialize};
use serde_json::Value;
use snafu::{Location, ResultExt, Snafu, location};
use std::collections::HashMap;

#[derive(Debug, Snafu)]
pub enum LLMError {
    #[snafu(display("Request failed"))]
    Request {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: reqwest::Error,
    },

    #[snafu(display("Request bad status"))]
    Api {
        status: u16,
        message: String,
        location: Location,
    },

    #[snafu(display("Request deserialize failed"))]
    Deserialize {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: serde_json::Error,
        text: String,
    },

    #[snafu(whatever, display("{message}"))]
    Whatever {
        message: String,
        #[snafu(source(from(Box<dyn std::error::Error + Send + Sync>, Some)))]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: Function,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub thinking: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantChatMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<Vec<ThinkingBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<Value>,
}

impl AssistantChatMessage {
    pub fn new_with_content(content: impl ToString) -> Self {
        AssistantChatMessage {
            content: Some(content.to_string()),
            tool_calls: None,
            reasoning_content: None,
            thinking_blocks: None,
            provider_specific_fields: None,
        }
    }

    pub fn new_with_tools(tool_calls: Vec<ToolCall>) -> Self {
        AssistantChatMessage {
            content: None,
            tool_calls: Some(tool_calls),
            reasoning_content: None,
            thinking_blocks: None,
            provider_specific_fields: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum ChatMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant(AssistantChatMessage),
    Tool {
        tool_call_id: String,
        name: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub total_tokens: i32,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: i32,
    pub message: ChatMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct LLMClient {
    base_url: String,
    client: reqwest::Client,
}

impl LLMClient {
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::new();
        Self { base_url, client }
    }

    pub async fn chat(
        &self,
        model: &str,
        system_prompt: &str,
        mut messages: Vec<ChatMessage>,
        tools: Option<Vec<Tool>>,
    ) -> Result<ChatResponse, LLMError> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        messages.insert(0, ChatMessage::system(system_prompt));

        let request = ChatRequest {
            model: model.to_string(),
            messages,
            tools,
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context(RequestSnafu)?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(LLMError::Api {
                status: status.as_u16(),
                message: error_text,
                location: location!(),
            });
        }

        let text = response.text().await.context(RequestSnafu)?;

        serde_json::from_str(&text).with_context(|_| DeserializeSnafu { text })
    }
}

// Helper functions for creating messages
impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self::System {
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::User {
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::Assistant(AssistantChatMessage {
            content: Some(content.into()),
            tool_calls: None,
            reasoning_content: None,
            thinking_blocks: None,
            provider_specific_fields: None,
        })
    }

    pub fn assistant_with_tools(tool_calls: Vec<ToolCall>) -> Self {
        Self::Assistant(AssistantChatMessage {
            content: None,
            tool_calls: Some(tool_calls),
            reasoning_content: None,
            thinking_blocks: None,
            provider_specific_fields: None,
        })
    }

    pub fn tool(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self::Tool {
            tool_call_id: tool_call_id.into(),
            name: name.into(),
            content: content.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_chat_message_serialization() {
        let system_msg = ChatMessage::system("You are helpful");
        let json = serde_json::to_value(&system_msg).unwrap();
        assert_eq!(json["role"], "system");
        assert_eq!(json["content"], "You are helpful");

        let user_msg = ChatMessage::user("Hello");
        let json = serde_json::to_value(&user_msg).unwrap();
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "Hello");

        let assistant_msg = ChatMessage::assistant("Hi there");
        let json = serde_json::to_value(&assistant_msg).unwrap();
        assert_eq!(json["role"], "assistant");
        assert_eq!(json["content"], "Hi there");

        let tool_msg = ChatMessage::tool("call_123", "test_tool", "result");
        let json = serde_json::to_value(&tool_msg).unwrap();
        assert_eq!(json["role"], "tool");
        assert_eq!(json["tool_call_id"], "call_123");
        assert_eq!(json["name"], "test_tool");
        assert_eq!(json["content"], "result");
    }

    #[test]
    fn test_chat_message_helpers() {
        let system_msg = ChatMessage::system("You are a helpful assistant");
        match system_msg {
            ChatMessage::System { content } => {
                assert_eq!(content, "You are a helpful assistant");
            }
            _ => panic!("Expected System message"),
        }

        let user_msg = ChatMessage::user("Hello!");
        match user_msg {
            ChatMessage::User { content } => {
                assert_eq!(content, "Hello!");
            }
            _ => panic!("Expected User message"),
        }

        let assistant_msg = ChatMessage::assistant("Hi there!");
        match assistant_msg {
            ChatMessage::Assistant(AssistantChatMessage {
                content,
                tool_calls,
                ..
            }) => {
                assert_eq!(content, Some("Hi there!".to_string()));
                assert!(tool_calls.is_none());
            }
            _ => panic!("Expected Assistant message"),
        }

        let tool_msg = ChatMessage::tool("call_123", "get_weather", "{\"temp\": 72}");
        match tool_msg {
            ChatMessage::Tool {
                tool_call_id,
                name,
                content,
            } => {
                assert_eq!(tool_call_id, "call_123");
                assert_eq!(name, "get_weather");
                assert_eq!(content, "{\"temp\": 72}");
            }
            _ => panic!("Expected Tool message"),
        }
    }

    #[test]
    fn test_tool_call_serialization() {
        let tool_call = ToolCall {
            id: "test_id".to_string(),
            tool_type: "function".to_string(),
            function: Function {
                name: "get_weather".to_string(),
                arguments: "{\"location\": \"SF\"}".to_string(),
            },
            index: Some(0),
        };

        let json = serde_json::to_value(&tool_call).unwrap();
        assert_eq!(json["id"], "test_id");
        assert_eq!(json["type"], "function");
        assert_eq!(json["function"]["name"], "get_weather");
        assert_eq!(json["function"]["arguments"], "{\"location\": \"SF\"}");
        assert_eq!(json["index"], 0);
    }

    #[test]
    fn test_chat_request_serialization() {
        let request = ChatRequest {
            model: "gpt-4".to_string(),
            messages: vec![
                ChatMessage::system("You are helpful"),
                ChatMessage::user("Hello"),
            ],
            tools: None,
        };

        let json_string = serde_json::to_string(&request).unwrap();
        let expected = r#"{"model":"gpt-4","messages":[{"role":"system","content":"You are helpful"},{"role":"user","content":"Hello"}]}"#;
        assert_eq!(json_string, expected);
    }

    #[test]
    fn test_chat_request_with_tools() {
        let tool = Tool {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: "get_weather".to_string(),
                description: "Get the weather".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    }
                }),
            },
        };

        let request = ChatRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage::user("What's the weather?")],
            tools: Some(vec![tool]),
        };

        let json_string = serde_json::to_string(&request).unwrap();
        let expected = r#"{"model":"gpt-4","messages":[{"role":"user","content":"What's the weather?"}],"tools":[{"type":"function","function":{"name":"get_weather","description":"Get the weather","parameters":{"properties":{"location":{"type":"string"}},"type":"object"}}}]}"#;
        assert_eq!(json_string, expected);
    }

    #[test]
    fn test_chat_response_deserialization() {
        let response_json = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1677652288,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello! How can I help you?"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "total_tokens": 30
            }
        });

        let response: ChatResponse = serde_json::from_value(response_json).unwrap();
        assert_eq!(response.id, "chatcmpl-123");
        assert_eq!(response.model, "gpt-4");
        assert_eq!(response.choices.len(), 1);
        match &response.choices[0].message {
            ChatMessage::Assistant(AssistantChatMessage { content, .. }) => {
                assert_eq!(content, &Some("Hello! How can I help you?".to_string()));
            }
            _ => panic!("Expected Assistant message"),
        }
        assert_eq!(response.usage.as_ref().unwrap().total_tokens, 30);
    }

    #[test]
    fn test_message_with_tool_calls() {
        let message_json = json!({
            "role": "assistant",
            "content": "I'll check the weather for you",
            "tool_calls": [{
                "id": "call_123",
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "arguments": "{\"location\": \"San Francisco\"}"
                }
            }],
            "reasoning_content": "The user wants weather information",
            "thinking_blocks": [{
                "type": "thinking",
                "thinking": "I need to call the weather function"
            }]
        });

        let message: ChatMessage = serde_json::from_value(message_json).unwrap();
        match message {
            ChatMessage::Assistant(AssistantChatMessage {
                content,
                tool_calls,
                reasoning_content,
                thinking_blocks,
                ..
            }) => {
                assert_eq!(content, Some("I'll check the weather for you".to_string()));
                assert!(tool_calls.is_some());
                assert_eq!(tool_calls.as_ref().unwrap().len(), 1);
                assert_eq!(tool_calls.as_ref().unwrap()[0].id, "call_123");
                assert_eq!(
                    reasoning_content,
                    Some("The user wants weather information".to_string())
                );
                assert!(thinking_blocks.is_some());
            }
            _ => panic!("Expected Assistant message"),
        }
    }

    #[test]
    fn test_llm_client_creation() {
        let client = LLMClient::new("http://localhost:4000".to_string());
        assert_eq!(client.base_url, "http://localhost:4000");

        let custom_client = LLMClient::new("http://custom:8080".to_string());
        assert_eq!(custom_client.base_url, "http://custom:8080");
    }

    #[tokio::test]
    async fn test_chat_request_building() {
        // This test just verifies the request can be built properly
        // In a real test environment, you'd mock the HTTP response
        let request = ChatRequest {
            model: "gpt-4".to_string(),
            messages: vec![
                ChatMessage::system("You are a helpful assistant"),
                ChatMessage::user("Hello, how are you?"),
            ],
            tools: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["model"], "gpt-4");
        assert_eq!(json["messages"].as_array().unwrap().len(), 2);
    }
}

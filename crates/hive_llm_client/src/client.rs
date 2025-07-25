use snafu::{Location, ResultExt, Snafu, location};

use crate::types::{ChatMessage, ChatRequest, ChatResponse, Tool};

#[derive(Debug, Snafu)]
pub enum LLMClientError {
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
    ) -> Result<ChatResponse, LLMClientError> {
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
            return Err(LLMClientError::Api {
                status: status.as_u16(),
                message: error_text,
                location: location!(),
            });
        }

        let text = response.text().await.context(RequestSnafu)?;

        serde_json::from_str(&text).with_context(|_| DeserializeSnafu { text })
    }
}

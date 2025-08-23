use std::time::Duration;

use reqwest::Method;
use wasmtime::component::Resource;

use crate::actors::manager::wasmind::actor::http;

use super::ActorState;

#[derive(Debug)]
pub struct HttpRequestResource {
    client: reqwest::Client,
    builder: reqwest::RequestBuilder,
    timeout_seconds: Option<u32>,
    retry_config: Option<RetryConfig>,
}

#[derive(Debug, Clone)]
pub struct RetryConfig {
    max_attempts: u32,
    base_delay_ms: u64,
    retryable_status_codes: Option<Vec<u16>>,
}

impl Clone for HttpRequestResource {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            builder: self.builder.try_clone().unwrap(),
            timeout_seconds: self.timeout_seconds,
            retry_config: self.retry_config.clone(),
        }
    }
}

impl http::Host for ActorState {}

impl http::HostRequest for ActorState {
    async fn new(
        &mut self,
        method: String,
        url: String,
    ) -> wasmtime::component::Resource<HttpRequestResource> {
        let client = reqwest::Client::new();

        let method = match Method::from_bytes(method.as_bytes()) {
            Ok(m) => m,
            Err(_) => Method::GET, // Default to GET if invalid method maybe error here in the future
        };

        let builder = client.request(method, url);

        let request_resource = HttpRequestResource {
            client,
            builder,
            timeout_seconds: None,
            retry_config: None,
        };

        self.table.push(request_resource).unwrap()
    }

    async fn header(
        &mut self,
        self_: Resource<HttpRequestResource>,
        key: String,
        value: String,
    ) -> Resource<HttpRequestResource> {
        let mut req = self.table.get(&self_).unwrap().clone();
        req.builder = req.builder.header(key, value);
        self.table.push(req).unwrap()
    }

    async fn headers(
        &mut self,
        self_: Resource<HttpRequestResource>,
        headers: http::Headers,
    ) -> Resource<HttpRequestResource> {
        let mut req = self.table.get(&self_).unwrap().clone();

        for (key, value) in headers.headers {
            req.builder = req.builder.header(key, value);
        }

        self.table.push(req).unwrap()
    }

    async fn body(
        &mut self,
        self_: Resource<HttpRequestResource>,
        body: Vec<u8>,
    ) -> Resource<HttpRequestResource> {
        let mut req = self.table.get(&self_).unwrap().clone();
        req.builder = req.builder.body(body);
        self.table.push(req).unwrap()
    }

    async fn timeout(
        &mut self,
        self_: Resource<HttpRequestResource>,
        seconds: u32,
    ) -> Resource<HttpRequestResource> {
        let mut req = self.table.get(&self_).unwrap().clone();
        req.timeout_seconds = Some(seconds);
        self.table.push(req).unwrap()
    }

    async fn retry(
        &mut self,
        self_: Resource<HttpRequestResource>,
        max_attempts: u32,
        base_delay_ms: u64,
    ) -> Resource<HttpRequestResource> {
        let mut req = self.table.get(&self_).unwrap().clone();
        req.retry_config = Some(RetryConfig {
            max_attempts,
            base_delay_ms,
            retryable_status_codes: None,
        });
        self.table.push(req).unwrap()
    }

    async fn retry_on_status_codes(
        &mut self,
        self_: Resource<HttpRequestResource>,
        codes: Vec<u16>,
    ) -> Resource<HttpRequestResource> {
        let mut req = self.table.get(&self_).unwrap().clone();

        // If there's existing retry config, update it; otherwise create a new one with defaults
        if let Some(ref mut retry_config) = req.retry_config {
            retry_config.retryable_status_codes = Some(codes);
        } else {
            // Create default retry config if none exists
            req.retry_config = Some(RetryConfig {
                max_attempts: 1, // No retries by default
                base_delay_ms: 1000,
                retryable_status_codes: Some(codes),
            });
        }

        self.table.push(req).unwrap()
    }

    async fn send(
        &mut self,
        self_: Resource<HttpRequestResource>,
    ) -> Result<http::Response, http::RequestError> {
        let req_resource = self
            .table
            .get_mut(&self_)
            .map_err(|e| http::RequestError::BuilderError(e.to_string()))?;

        let retry_config = req_resource.retry_config.clone();
        let timeout_seconds = req_resource.timeout_seconds;

        // Determine max attempts (default to 1 if no retry config)
        let max_attempts = retry_config.as_ref().map(|c| c.max_attempts).unwrap_or(1);

        let mut last_error = None;

        for attempt in 0..max_attempts {
            let mut builder = req_resource.builder.try_clone().unwrap();

            if let Some(timeout_seconds) = timeout_seconds {
                builder = builder.timeout(Duration::from_secs(timeout_seconds as u64));
            }

            match builder.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();

                    // Check if this status code should trigger a retry
                    let should_retry = if let Some(ref retry_config) = retry_config {
                        if let Some(ref retryable_codes) = retry_config.retryable_status_codes {
                            retryable_codes.contains(&status)
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    // If we should retry and this isn't the last attempt, continue to next iteration
                    if should_retry && attempt < max_attempts - 1 {
                        // Apply exponential backoff delay
                        if let Some(retry_config) = retry_config.as_ref() {
                            let delay_ms = retry_config.base_delay_ms * (2_u64.pow(attempt));
                            let delay = Duration::from_millis(delay_ms);

                            tracing::info!(
                                "HTTP request failed with status {} (attempt {}/{}), retrying in {}ms",
                                status,
                                attempt + 1,
                                max_attempts,
                                delay_ms
                            );

                            tokio::time::sleep(delay).await;
                        }

                        continue; // Retry the request
                    }

                    // Convert headers
                    let mut headers_vec = Vec::new();
                    for (name, value) in resp.headers() {
                        if let Ok(value_str) = value.to_str() {
                            headers_vec.push((name.to_string(), value_str.to_string()));
                        }
                    }
                    let headers = http::Headers {
                        headers: headers_vec,
                    };

                    let body = match resp.bytes().await {
                        Ok(bytes) => bytes.to_vec(),
                        Err(e) => return Err(http::RequestError::NetworkError(e.to_string())),
                    };

                    return Ok(http::Response {
                        status,
                        headers,
                        body,
                    });
                }
                Err(e) => {
                    last_error = Some(e);

                    // If this is not the last attempt and we have retry config, wait before retrying
                    if attempt < max_attempts - 1
                        && let Some(retry_config) = retry_config.as_ref()
                    {
                        // Exponential backoff: delay = base_delay * 2^attempt
                        let delay_ms = retry_config.base_delay_ms * (2_u64.pow(attempt));
                        let delay = Duration::from_millis(delay_ms);

                        tracing::info!(
                            "HTTP request failed with network error (attempt {}/{}), retrying in {}ms",
                            attempt + 1,
                            max_attempts,
                            delay_ms
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        // All attempts failed, send notification and return the last error
        let e = last_error.unwrap();

        // Send toast notification about retry failure
        if max_attempts > 1 {
            use wasmind_actor_utils::common_messages::ui::{NotificationLevel, UserNotification};
            if let Ok(notification) = serde_json::to_string(&UserNotification {
                level: NotificationLevel::Error,
                title: "HTTP Request Failed".to_string(),
                message: format!("Request failed after {max_attempts} attempts"),
                source: Some("HTTP".to_string()),
            }) {
                let _ = self.tx.send(crate::actors::MessageEnvelope {
                    id: "http-retry-failure".to_string(),
                    message_type: "wasmind.common.ui.UserNotification".to_string(),
                    from_actor_id: self.actor_id.clone(),
                    from_scope: self.scope.clone(),
                    payload: notification.into_bytes(),
                });
            }
        }

        if e.is_timeout() {
            Err(http::RequestError::Timeout)
        } else if e.is_builder() {
            Err(http::RequestError::BuilderError(e.to_string()))
        } else if let Some(url) = e.url() {
            Err(http::RequestError::InvalidUrl(url.to_string()))
        } else {
            Err(http::RequestError::NetworkError(e.to_string()))
        }
    }

    async fn drop(&mut self, self_: Resource<HttpRequestResource>) -> wasmtime::Result<()> {
        self.table.delete(self_)?;
        Ok(())
    }
}

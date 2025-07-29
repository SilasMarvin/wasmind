use std::time::Duration;

use reqwest::Method;
use wasmtime::component::Resource;

use crate::actors::manager::hive::actor::http;

use super::ActorState;

#[derive(Debug)]
pub struct HttpRequestResource {
    client: reqwest::Client,
    builder: reqwest::RequestBuilder,
    timeout_seconds: Option<u32>,
}

impl Clone for HttpRequestResource {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            builder: self.builder.try_clone().unwrap(),
            timeout_seconds: self.timeout_seconds.clone(),
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
        // Create a new client for each request (could optimize with a shared client later)
        let client = reqwest::Client::new();

        // Parse method
        let method = match Method::from_bytes(method.as_bytes()) {
            Ok(m) => m,
            Err(_) => Method::GET, // Default to GET if invalid method
        };

        // Create the request builder
        let builder = client.request(method, url);

        let request_resource = HttpRequestResource {
            client,
            builder,
            timeout_seconds: None,
        };

        let resource = self.table.push(request_resource).unwrap();
        resource
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

    async fn send(
        &mut self,
        self_: Resource<HttpRequestResource>,
    ) -> Result<http::Response, http::RequestError> {
        let req_resource = self
            .table
            .get_mut(&self_)
            .map_err(|e| http::RequestError::BuilderError(e.to_string()))?;

        let mut builder = req_resource.builder.try_clone().unwrap();

        // Apply timeout if set
        if let Some(timeout_seconds) = req_resource.timeout_seconds {
            builder = builder.timeout(Duration::from_secs(timeout_seconds as u64));
        }

        // Send the request
        let response = match builder.send().await {
            Ok(resp) => resp,
            Err(e) => {
                if e.is_timeout() {
                    return Err(http::RequestError::Timeout);
                } else if e.is_builder() {
                    return Err(http::RequestError::BuilderError(e.to_string()));
                } else if let Some(url) = e.url() {
                    return Err(http::RequestError::InvalidUrl(url.to_string()));
                } else {
                    return Err(http::RequestError::NetworkError(e.to_string()));
                }
            }
        };

        // Extract response parts
        let status = response.status().as_u16();

        // Convert headers
        let mut headers_vec = Vec::new();
        for (name, value) in response.headers() {
            if let Ok(value_str) = value.to_str() {
                headers_vec.push((name.to_string(), value_str.to_string()));
            }
        }
        let headers = http::Headers {
            headers: headers_vec,
        };

        // Get body
        let body = match response.bytes().await {
            Ok(bytes) => bytes.to_vec(),
            Err(e) => return Err(http::RequestError::NetworkError(e.to_string())),
        };

        Ok(http::Response {
            status,
            headers,
            body,
        })
    }

    async fn drop(&mut self, self_: Resource<HttpRequestResource>) -> wasmtime::Result<()> {
        self.table.delete(self_)?;
        Ok(())
    }
}


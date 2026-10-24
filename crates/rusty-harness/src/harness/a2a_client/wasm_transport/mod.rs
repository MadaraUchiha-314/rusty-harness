//! A JSON-RPC transport over the host's `HttpClient` for `wasm32`, where the official
//! client crate does not build (decision-003). Uses the official `a2a` types; the
//! streaming call is sent as `SendMessage` and its one response replayed as a stream,
//! which is what the browser's own renderer does through `@a2a-js/sdk` anyway (M5).

use std::sync::Arc;

use serde_json::{Value, json};

use super::{AgentTransport, ResponseStream};
use crate::harness_async_trait;
use crate::host::{HttpClient, HttpRequest};
use crate::runtime::boxed_stream;

/// JSON-RPC over the host's HTTP client.
pub struct HttpJsonRpcTransport {
    endpoint: url::Url,
    http: Arc<dyn HttpClient>,
    bearer: Option<String>,
}

impl core::fmt::Debug for HttpJsonRpcTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HttpJsonRpcTransport")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl HttpJsonRpcTransport {
    /// A transport to `endpoint`.
    #[must_use]
    pub fn new(endpoint: url::Url, http: Arc<dyn HttpClient>, bearer: Option<String>) -> Self {
        Self {
            endpoint,
            http,
            bearer,
        }
    }

    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
        extensions: &[String],
    ) -> Result<T, a2a::A2AError> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let mut request = HttpRequest::new(http::Method::POST, self.endpoint.clone());
        request.headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        request
            .headers
            .insert("A2A-Version", http::HeaderValue::from_static("1.0"));
        if !extensions.is_empty()
            && let Ok(v) = http::HeaderValue::from_str(&extensions.join(", "))
        {
            request.headers.insert("A2A-Extensions", v);
        }
        if let Some(b) = &self.bearer
            && let Ok(v) = http::HeaderValue::from_str(&format!("Bearer {b}"))
        {
            request.headers.insert(http::header::AUTHORIZATION, v);
        }
        request.body = bytes::Bytes::from(body.to_string());
        let response = self
            .http
            .send(request)
            .await
            .map_err(|e| a2a::A2AError::internal(e.to_string()))?;
        let envelope: Value = serde_json::from_slice(&response.body)
            .map_err(|e| a2a::A2AError::parse_error(e.to_string()))?;
        if let Some(err) = envelope.get("error") {
            return Err(a2a::A2AError::new(
                err.get("code")
                    .and_then(Value::as_i64)
                    .and_then(|c| i32::try_from(c).ok())
                    .unwrap_or(-32603),
                err.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_owned(),
            ));
        }
        serde_json::from_value(envelope.get("result").cloned().unwrap_or(Value::Null))
            .map_err(|e| a2a::A2AError::invalid_agent_response_with(e.to_string()))
    }
}

trait InvalidWith {
    fn invalid_agent_response_with(reason: String) -> Self;
}

impl InvalidWith for a2a::A2AError {
    fn invalid_agent_response_with(reason: String) -> Self {
        Self::internal(format!("invalid agent response: {reason}"))
    }
}

harness_async_trait! {
    impl AgentTransport for HttpJsonRpcTransport {
        async fn send_streaming_message(&self, request: a2a::SendMessageRequest, extensions: Vec<String>) -> Result<ResponseStream, a2a::A2AError> {
            let params = serde_json::to_value(&request).map_err(|e| a2a::A2AError::internal(e.to_string()))?;
            let response: a2a::SendMessageResponse = self.call("SendMessage", params, &extensions).await?;
            let event = match response {
                a2a::SendMessageResponse::Task(t) => a2a::StreamResponse::Task(t),
                a2a::SendMessageResponse::Message(m) => a2a::StreamResponse::Message(m),
            };
            Ok(boxed_stream(futures::stream::iter(vec![Ok(event)])))
        }

        async fn get_task(&self, request: a2a::GetTaskRequest) -> Result<a2a::Task, a2a::A2AError> {
            let params = serde_json::to_value(&request).map_err(|e| a2a::A2AError::internal(e.to_string()))?;
            self.call("GetTask", params, &[]).await
        }

        async fn cancel_task(&self, request: a2a::CancelTaskRequest) -> Result<a2a::Task, a2a::A2AError> {
            let params = serde_json::to_value(&request).map_err(|e| a2a::A2AError::internal(e.to_string()))?;
            self.call("CancelTask", params, &[]).await
        }
    }
}

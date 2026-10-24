//! The official client crate as the native transport (decision-003): JSON-RPC over the
//! card's interface, a bearer credential from a `SecretRef` when configured, and the card
//! fetched through the host's HTTP client so the testing host can serve it.

use std::sync::Arc;

use a2a_client::A2AClient;
use a2a_client::auth::AuthInterceptor;
use a2a_client::jsonrpc::JsonRpcTransport;
use a2a_client::middleware::CallInterceptor;

use super::{AgentTransport, CardError, ResponseStream, jsonrpc_interface};
use crate::harness_async_trait;
use crate::host::{Host, HttpRequest, Secret};
use crate::runtime::boxed_stream;

/// The well-known path of an agent card.
pub const CARD_PATH: &str = "/.well-known/agent-card.json";

/// Fetches and parses the card at `base_url` through the host's HTTP client (R17.1).
///
/// # Errors
///
/// [`CardError::Fetch`] or [`CardError::Parse`]; validation is the caller's
/// (`RemoteAgent::new`).
pub async fn fetch_card(host: &Host, base_url: &url::Url) -> Result<a2a::AgentCard, CardError> {
    let http = host.http().map_err(|e| CardError::Fetch {
        url: base_url.to_string(),
        reason: e.to_string(),
    })?;
    let mut url = base_url.clone();
    if !url.path().ends_with("agent-card.json") {
        url.set_path(CARD_PATH);
    }
    let response = http
        .send(HttpRequest::new(http::Method::GET, url.clone()))
        .await
        .map_err(|e| CardError::Fetch {
            url: url.to_string(),
            reason: e.to_string(),
        })?;
    if !response.status.is_success() {
        return Err(CardError::Fetch {
            url: url.to_string(),
            reason: format!("http {}", response.status.as_u16()),
        });
    }
    serde_json::from_slice(&response.body).map_err(|e| CardError::Parse(e.to_string()))
}

/// The official client over the card's JSON-RPC interface.
pub struct OfficialTransport {
    client: A2AClient<JsonRpcTransport>,
    endpoint: String,
    extensions: Vec<String>,
}

/// Activates the harness's extensions on every call (`A2A-Extensions`, R19.3).
struct ExtensionsInterceptor(Vec<String>);

#[async_trait::async_trait]
impl CallInterceptor for ExtensionsInterceptor {
    async fn before(
        &self,
        _method: &str,
        params: &mut a2a_client::ServiceParams,
    ) -> Result<(), a2a::A2AError> {
        if !self.0.is_empty() {
            params.insert("A2A-Extensions".into(), vec![self.0.join(", ")]);
        }
        Ok(())
    }
}

impl core::fmt::Debug for OfficialTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OfficialTransport")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl OfficialTransport {
    /// A transport for `card`'s JSON-RPC interface, presenting `bearer` when given. The
    /// secret is consumed here and never stored elsewhere (boundary 6).
    ///
    /// # Errors
    ///
    /// [`CardError::Invalid`] when the card has no JSON-RPC interface.
    pub fn from_card(
        card: &a2a::AgentCard,
        bearer: Option<Secret<String>>,
        extensions: Vec<String>,
    ) -> Result<Self, CardError> {
        let iface = jsonrpc_interface(card).ok_or_else(|| CardError::Invalid {
            name: card.name.clone(),
            reason: "no JSONRPC interface".into(),
        })?;
        let endpoint = iface.wire_url();
        let mut interceptors: Vec<Arc<dyn CallInterceptor>> =
            vec![Arc::new(ExtensionsInterceptor(extensions.clone()))];
        if let Some(secret) = bearer {
            interceptors.push(Arc::new(AuthInterceptor::bearer(
                secret.expose().to_owned(),
            )));
        }
        let client = A2AClient::new(JsonRpcTransport::new(
            reqwest::Client::new(),
            endpoint.clone(),
        ))
        .with_interceptors(interceptors);
        Ok(Self {
            client,
            endpoint,
            extensions,
        })
    }

    /// The extensions every call activates.
    #[must_use]
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }
}

harness_async_trait! {
    impl AgentTransport for OfficialTransport {
        async fn send_streaming_message(&self, request: a2a::SendMessageRequest, _extensions: Vec<String>) -> Result<ResponseStream, a2a::A2AError> {
            // The active set is fixed per client (the interceptor); a per-call list is
            // the loopback and browser transports' concern.
            let stream = self.client.send_streaming_message(&request).await?;
            Ok(boxed_stream(stream))
        }

        async fn get_task(&self, request: a2a::GetTaskRequest) -> Result<a2a::Task, a2a::A2AError> {
            self.client.get_task(&request).await
        }

        async fn cancel_task(&self, request: a2a::CancelTaskRequest) -> Result<a2a::Task, a2a::A2AError> {
            self.client.cancel_task(&request).await
        }
    }
}

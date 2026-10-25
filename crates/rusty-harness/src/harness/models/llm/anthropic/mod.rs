//! The Anthropic client over the provider's HTTP API, with prompt-cache breakpoints
//! (R9.1).
//!
//! The Messages API with streaming, tool use and image input; `cache_control` markers go
//! on the system block and on the last message before each [`CacheBreakpoint`] the
//! context builder set.

use std::collections::BTreeSet;
use std::sync::Arc;

use base64::Engine as _;
use bytes::Bytes;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use super::sse;
use super::{
    CacheBreakpoint, ContentPart, EventStream, Llm, MessageRole, Modality, ModelCapabilities,
    ModelError, ModelEvent, ModelRequest, StopReason, Usage, check_modalities, retry_after,
};
use crate::config::ModelConfig;
use crate::harness::tools::ToolCall;
use crate::harness_async_trait;
use crate::host::{Host, HttpClient, HttpRequest, Secret};
use crate::registry::{Entity, EntityId, EntityKind, Placement, RemoteEndpoint};

/// The default base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// The API version header the client sends.
pub const API_VERSION: &str = "2023-06-01";

/// The default output cap: the Messages API requires one.
pub const DEFAULT_MAX_TOKENS: u32 = 8192;

/// A model served through Anthropic's Messages API.
pub struct AnthropicLlm {
    id: EntityId,
    model: String,
    base_url: url::Url,
    api_key: Secret<String>,
    http: Arc<dyn HttpClient>,
    caps: ModelCapabilities,
}

impl core::fmt::Debug for AnthropicLlm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnthropicLlm")
            .field("id", &self.id)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl AnthropicLlm {
    /// Builds the client for `[models.<alias>]`, resolving the credential through the host.
    ///
    /// # Errors
    ///
    /// [`ModelError::NonRetryable`] when the credential cannot be resolved or the host
    /// has no HTTP client.
    pub async fn from_config(
        alias: &str,
        config: &ModelConfig,
        host: &Host,
    ) -> Result<Self, ModelError> {
        let secrets = host.secrets().map_err(|e| ModelError::NonRetryable {
            status: None,
            message: e.to_string(),
        })?;
        let api_key =
            secrets
                .resolve(&config.api_key)
                .await
                .map_err(|e| ModelError::NonRetryable {
                    status: None,
                    message: e.to_string(),
                })?;
        let http = host.http().map_err(|e| ModelError::NonRetryable {
            status: None,
            message: e.to_string(),
        })?;
        let base_url = config.base_url.clone().map_or_else(
            || {
                DEFAULT_BASE_URL
                    .parse()
                    .map_err(|_| ModelError::Decode("default base url".into()))
            },
            Ok,
        )?;
        Ok(Self::new(alias, &config.model, base_url, api_key, http))
    }

    /// Builds the client from its parts.
    #[must_use]
    pub fn new(
        alias: &str,
        model: &str,
        base_url: url::Url,
        api_key: Secret<String>,
        http: Arc<dyn HttpClient>,
    ) -> Self {
        Self {
            id: EntityId::new(alias),
            model: model.to_owned(),
            base_url,
            api_key,
            http,
            caps: ModelCapabilities {
                input_modalities: BTreeSet::from([Modality::Text, Modality::Image]),
                tools: true,
                caching: true,
                max_window_tokens: 200_000,
                effort_levels: vec![],
            },
        }
    }

    /// The Messages request body for `request` (snapshot-tested).
    #[must_use]
    pub fn body(&self, request: &ModelRequest) -> Value {
        let cached_after: BTreeSet<usize> = request
            .cache_hints
            .iter()
            .map(|CacheBreakpoint { after }| *after)
            .collect();
        let mut system = json!({ "type": "text", "text": request.instructions.render() });
        if cached_after.contains(&0) {
            system["cache_control"] = json!({ "type": "ephemeral" });
        }
        let mut messages = Vec::new();
        for (index, message) in request.messages.iter().enumerate() {
            let mut content: Vec<Value> = match message.role {
                MessageRole::Tool => vec![
                    json!({ "type": "tool_result", "tool_use_id": message.tool_call_id, "content": text_of(&message.parts) }),
                ],
                _ => parts(&message.parts),
            };
            for call in &message.tool_calls {
                content.push(json!({ "type": "tool_use", "id": call.id, "name": call.name, "input": call.arguments }));
            }
            if cached_after.contains(&(index + 1))
                && let Some(last) = content.last_mut()
            {
                last["cache_control"] = json!({ "type": "ephemeral" });
            }
            let role = match message.role {
                MessageRole::Assistant => "assistant",
                MessageRole::User | MessageRole::Tool => "user",
            };
            messages.push(json!({ "role": role, "content": content }));
        }
        let mut body = json!({
            "model": self.model,
            "system": [system],
            "messages": messages,
            "max_tokens": request.max_output_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            "stream": true,
        });
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request.tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.parameters })).collect(),
            );
        }
        body
    }

    fn endpoint(&self) -> url::Url {
        let mut url = self.base_url.clone();
        let path = format!("{}/v1/messages", url.path().trim_end_matches('/'));
        url.set_path(&path);
        url
    }
}

fn parts(parts: &[ContentPart]) -> Vec<Value> {
    parts
        .iter()
        .map(|p| match p {
            ContentPart::Text { text } => json!({ "type": "text", "text": text }),
            ContentPart::Image { media_type, data } => json!({
                "type": "image",
                "source": { "type": "base64", "media_type": media_type, "data": base64::engine::general_purpose::STANDARD.encode(data) }
            }),
            ContentPart::Audio { media_type, .. } => json!({ "type": "text", "text": format!("[audio {media_type} omitted: not accepted by this model]") }),
        })
        .collect()
}

fn text_of(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|p| {
            if let ContentPart::Text { text } = p {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Entity for AnthropicLlm {
    fn kind(&self) -> EntityKind {
        EntityKind::Llm
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
    fn placement(&self) -> Placement {
        Placement::Remote(RemoteEndpoint::Http {
            base_url: self.base_url.clone(),
        })
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event {
    MessageStart {
        message: MessageStart,
    },
    ContentBlockStart {
        index: usize,
        content_block: Block,
    },
    ContentBlockDelta {
        index: usize,
        delta: BlockDelta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        delta: MessageDelta,
        #[serde(default)]
        usage: Option<DeltaUsage>,
    },
    MessageStop,
    Ping,
    Error {
        error: ApiError,
    },
}

#[derive(Deserialize)]
struct MessageStart {
    #[serde(default)]
    usage: Option<StartUsage>,
}

#[derive(Deserialize)]
struct StartUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    Text {
        #[serde(default)]
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BlockDelta {
    TextDelta {
        text: String,
    },
    InputJsonDelta {
        partial_json: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct MessageDelta {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct DeltaUsage {
    #[serde(default)]
    output_tokens: u64,
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

#[derive(Default)]
struct Partial {
    id: String,
    name: String,
    json: String,
}

/// Folds Messages API events into model events.
#[derive(Default)]
struct Decoder {
    tools: std::collections::BTreeMap<usize, Partial>,
    usage: Usage,
    stop: Option<StopReason>,
}

impl Decoder {
    fn feed(&mut self, data: &str) -> Result<Vec<ModelEvent>, ModelError> {
        let parsed: Event =
            serde_json::from_str(data).map_err(|e| ModelError::Decode(e.to_string()))?;
        let mut out = Vec::new();
        match parsed {
            Event::MessageStart { message } => {
                if let Some(u) = message.usage {
                    self.usage.input_tokens = u.input_tokens;
                    self.usage.cached_input_tokens = u.cache_read_input_tokens;
                }
            }
            Event::ContentBlockStart {
                index,
                content_block,
            } => self.block_start(index, content_block, &mut out),
            Event::ContentBlockDelta { index, delta } => self.block_delta(index, delta, &mut out),
            Event::ContentBlockStop { index } => {
                if let Some(p) = self.tools.remove(&index) {
                    let arguments = if p.json.trim().is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(&p.json).unwrap_or(json!({}))
                    };
                    out.push(ModelEvent::ToolCall {
                        call: ToolCall {
                            id: p.id,
                            name: p.name,
                            arguments,
                        },
                    });
                }
            }
            Event::MessageDelta { delta, usage } => {
                if let Some(u) = usage {
                    self.usage.output_tokens = u.output_tokens;
                }
                if let Some(reason) = delta.stop_reason {
                    self.stop = Some(match reason.as_str() {
                        "end_turn" | "stop_sequence" => StopReason::EndTurn,
                        "tool_use" => StopReason::ToolUse,
                        "max_tokens" => StopReason::MaxTokens,
                        other => StopReason::Other(other.to_owned()),
                    });
                }
            }
            Event::MessageStop | Event::Ping => {}
            Event::Error { error } => {
                return Err(ModelError::Retryable {
                    status: None,
                    message: error.message,
                    retry_after: None,
                });
            }
        }
        Ok(out)
    }

    fn block_start(&mut self, index: usize, block: Block, out: &mut Vec<ModelEvent>) {
        match block {
            Block::Text { text } if !text.is_empty() => out.push(ModelEvent::TextDelta { text }),
            Block::ToolUse { id, name } => {
                self.tools.insert(
                    index,
                    Partial {
                        id,
                        name,
                        json: String::new(),
                    },
                );
            }
            Block::Text { .. } | Block::Other => {}
        }
    }

    fn block_delta(&mut self, index: usize, delta: BlockDelta, out: &mut Vec<ModelEvent>) {
        match delta {
            BlockDelta::TextDelta { text } => out.push(ModelEvent::TextDelta { text }),
            BlockDelta::InputJsonDelta { partial_json } => {
                if let Some(p) = self.tools.get_mut(&index) {
                    p.json.push_str(&partial_json);
                }
            }
            BlockDelta::Other => {}
        }
    }

    fn finish(&mut self) -> Vec<ModelEvent> {
        vec![
            ModelEvent::Usage { usage: self.usage },
            ModelEvent::Stop {
                reason: self.stop.take().unwrap_or(StopReason::EndTurn),
            },
        ]
    }
}

/// Turns Messages API SSE events into model events.
pub fn decode(bytes: crate::host::ByteStream) -> EventStream<'static> {
    let stream = futures::stream::unfold(
        (sse::events(bytes), Decoder::default(), false),
        |(mut sse, mut decoder, ended)| async move {
            if ended {
                return None;
            }
            Some(match sse.next().await {
                None => (
                    decoder.finish().into_iter().map(Ok).collect::<Vec<_>>(),
                    (sse, decoder, true),
                ),
                Some(Err(err)) => (
                    vec![Err(ModelError::Retryable {
                        status: None,
                        message: err.to_string(),
                        retry_after: None,
                    })],
                    (sse, decoder, true),
                ),
                Some(Ok(event)) => match decoder.feed(&event.data) {
                    Ok(events) => (events.into_iter().map(Ok).collect(), (sse, decoder, false)),
                    Err(err) => (vec![Err(err)], (sse, decoder, true)),
                },
            })
        },
    )
    .flat_map(futures::stream::iter);
    crate::runtime::boxed_stream(stream)
}

harness_async_trait! {
    impl Llm for AnthropicLlm {
        fn capabilities(&self) -> &ModelCapabilities {
            &self.caps
        }

        async fn stream(&self, request: ModelRequest) -> Result<EventStream<'static>, ModelError> {
            check_modalities(&self.caps, &request)?;
            let mut http_request = HttpRequest::new(http::Method::POST, self.endpoint());
            http_request.headers.insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("application/json"));
            http_request.headers.insert("anthropic-version", http::HeaderValue::from_static(API_VERSION));
            let key = http::HeaderValue::from_str(self.api_key.expose())
                .map_err(|_| ModelError::NonRetryable { status: None, message: "credential is not a valid header value".into() })?;
            http_request.headers.insert("x-api-key", key);
            http_request.body = Bytes::from(serde_json::to_vec(&self.body(&request)).map_err(|e| ModelError::Decode(e.to_string()))?);
            let response = self
                .http
                .send_streaming(http_request)
                .await
                .map_err(|e| ModelError::Retryable { status: None, message: e.to_string(), retry_after: None })?;
            if !response.status.is_success() {
                let mut body = response.body;
                let mut text = Vec::new();
                while let Some(Ok(chunk)) = body.next().await {
                    text.extend_from_slice(&chunk);
                    if text.len() > 4096 {
                        break;
                    }
                }
                return Err(ModelError::from_status(response.status.as_u16(), String::from_utf8_lossy(&text).replace(self.api_key.expose(), "[redacted]"), retry_after(&response.headers)));
            }
            Ok(decode(response.body))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AnthropicLlm;
    use crate::assets::{DataRole, Prompt};
    use crate::harness::models::llm::{
        CacheBreakpoint, ContextMessage, Llm, MessageRole, ModelRequest, StopReason, collect,
    };
    use crate::harness::tools::{Idempotency, ToolDescriptor};
    use crate::host::testing::MockHttp;
    use crate::host::{HttpResponse, Secret};
    use std::sync::Arc;

    fn request() -> ModelRequest {
        let mut req = ModelRequest::new(
            Prompt::data(DataRole::TaskField, "be brief"),
            vec![
                ContextMessage::text(MessageRole::User, "hi"),
                ContextMessage::text(MessageRole::Assistant, "hello"),
                ContextMessage::tool_result("toolu_1", "42"),
            ],
        );
        req.messages[1]
            .tool_calls
            .push(crate::harness::tools::ToolCall {
                id: "toolu_1".into(),
                name: "add".into(),
                arguments: serde_json::json!({"a": 1}),
            });
        req.tools.push(ToolDescriptor {
            name: "add".into(),
            description: "adds".into(),
            parameters: serde_json::json!({"type": "object"}),
            idempotency: Idempotency::Idempotent,
        });
        req.cache_hints = vec![CacheBreakpoint { after: 0 }, CacheBreakpoint { after: 1 }];
        req
    }

    fn client(http: Arc<MockHttp>) -> AnthropicLlm {
        AnthropicLlm::new(
            "claude",
            "claude-fable-5-1",
            "https://api.anthropic.com".parse().unwrap(),
            Secret::new("sk-ant-canary".into()),
            http,
        )
    }

    #[test]
    fn snapshot_anthropic_request_body() {
        let body = client(Arc::new(MockHttp::always(http::StatusCode::OK, ""))).body(&request());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/snapshots/anthropic_request_body.json");
        let json = serde_json::to_string_pretty(&body).unwrap() + "\n";
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            std::fs::write(&path, &json).unwrap();
        }
        let published: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default())
                .expect("snapshot is JSON");
        assert_eq!(
            published, body,
            "anthropic_request_body.json is stale: rerun with UPDATE_SNAPSHOTS=1"
        );
        assert_eq!(
            body["system"][0]["cache_control"]["type"], "ephemeral",
            "breakpoint after the instructions"
        );
        assert_eq!(
            body["messages"][0]["content"][0]["cache_control"]["type"], "ephemeral",
            "breakpoint after message 1"
        );
        assert!(
            body["messages"][1]["content"][0]
                .get("cache_control")
                .is_none()
        );
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
    }

    #[tokio::test]
    async fn streams_text_tool_use_and_usage_with_the_api_key_header() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let http = Arc::new(MockHttp::new(move |req| {
            seen2.lock().unwrap().push(
                req.headers
                    .get("x-api-key")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_owned(),
            );
            Ok(HttpResponse { status: http::StatusCode::OK, headers: http::HeaderMap::new(), body: concat!(
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"cache_read_input_tokens\":9}}}\n\n",
                "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n",
                "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
                "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_9\",\"name\":\"add\"}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"a\\\":\"}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"2}\"}}\n\n",
                "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":5}}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n").into() })
        }));
        let turn = collect(client(http).stream(request()).await.expect("stream"), None)
            .await
            .expect("turn");
        assert_eq!(turn.text, "Hi");
        assert_eq!(turn.tool_calls[0].arguments, serde_json::json!({"a": 2}));
        assert_eq!(turn.stop, Some(StopReason::ToolUse));
        let usage = turn.usage.unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cached_input_tokens
            ),
            (12, 5, 9)
        );
        assert_eq!(seen.lock().unwrap().as_slice(), ["sk-ant-canary"]);
    }
}

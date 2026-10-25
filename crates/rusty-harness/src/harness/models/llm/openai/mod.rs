//! The `OpenAI` client over the provider's HTTP API; a configurable base URL serves
//! `OpenAI`-compatible providers (R9.3).
//!
//! Chat Completions with streaming, tool calling, image and audio input and
//! `reasoning_effort`; prompt caching is automatic on `OpenAI`'s side, which the context
//! builder's stable prefix serves.

use std::collections::BTreeSet;
use std::sync::Arc;

use base64::Engine as _;
use bytes::Bytes;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use super::sse;
use super::{
    ContentPart, EventStream, Llm, MessageRole, Modality, ModelCapabilities, ModelError,
    ModelEvent, ModelRequest, StopReason, Usage, WireNames, check_modalities, retry_after,
};
use crate::config::ModelConfig;
use crate::harness::tools::ToolCall;
use crate::harness_async_trait;
use crate::host::{Host, HttpClient, HttpRequest, Secret};
use crate::registry::{Entity, EntityId, EntityKind, Placement, RemoteEndpoint};

/// The default base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// A model served through `OpenAI`'s Chat Completions API.
pub struct OpenAiLlm {
    id: EntityId,
    model: String,
    base_url: url::Url,
    api_key: Secret<String>,
    http: Arc<dyn HttpClient>,
    caps: ModelCapabilities,
}

impl core::fmt::Debug for OpenAiLlm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpenAiLlm")
            .field("id", &self.id)
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl OpenAiLlm {
    /// Builds the client for `[models.<alias>]`, resolving the credential through the host.
    ///
    /// # Errors
    ///
    /// [`ModelError::NonRetryable`] when the credential reference cannot be resolved or
    /// the host has no HTTP client.
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
                input_modalities: BTreeSet::from([
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                ]),
                tools: true,
                caching: true,
                max_window_tokens: 400_000,
                effort_levels: vec![
                    super::Effort::Low,
                    super::Effort::Medium,
                    super::Effort::High,
                ],
            },
        }
    }

    /// The Chat Completions request body for `request` (snapshot-tested).
    #[must_use]
    pub fn body(&self, request: &ModelRequest) -> Value {
        let names = WireNames::new(&request.tools);
        let mut messages =
            vec![json!({ "role": "system", "content": request.instructions.render() })];
        for message in &request.messages {
            messages.push(match message.role {
                MessageRole::User => json!({ "role": "user", "content": parts(&message.parts) }),
                MessageRole::Assistant => {
                    let mut m = json!({ "role": "assistant", "content": text_of(&message.parts) });
                    if !message.tool_calls.is_empty() {
                        m["tool_calls"] = Value::Array(
                            message
                                .tool_calls
                                .iter()
                                .map(|c| json!({ "id": c.id, "type": "function", "function": { "name": names.wire(&c.name), "arguments": c.arguments.to_string() } }))
                                .collect(),
                        );
                    }
                    m
                }
                MessageRole::Tool => json!({ "role": "tool", "tool_call_id": message.tool_call_id, "content": text_of(&message.parts) }),
            });
        }
        let mut body = json!({ "model": self.model, "messages": messages, "stream": true, "stream_options": { "include_usage": true } });
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|t| json!({ "type": "function", "function": { "name": names.wire(&t.name), "description": t.description, "parameters": t.parameters } }))
                    .collect(),
            );
        }
        if let Some(effort) = request.effort {
            body["reasoning_effort"] = json!(effort.to_string());
        }
        if let Some(max) = request.max_output_tokens {
            body["max_completion_tokens"] = json!(max);
        }
        body
    }

    fn endpoint(&self) -> url::Url {
        let mut url = self.base_url.clone();
        let path = format!("{}/chat/completions", url.path().trim_end_matches('/'));
        url.set_path(&path);
        url
    }
}

fn parts(parts: &[ContentPart]) -> Value {
    Value::Array(
        parts
            .iter()
            .map(|p| match p {
                ContentPart::Text { text } => json!({ "type": "text", "text": text }),
                ContentPart::Image { media_type, data } => {
                    json!({ "type": "image_url", "image_url": { "url": format!("data:{media_type};base64,{}", base64::engine::general_purpose::STANDARD.encode(data)) } })
                }
                ContentPart::Audio { media_type, data } => json!({
                    "type": "input_audio",
                    "input_audio": { "data": base64::engine::general_purpose::STANDARD.encode(data), "format": media_type.strip_prefix("audio/").unwrap_or(media_type) }
                }),
            })
            .collect(),
    )
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

impl Entity for OpenAiLlm {
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
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<ChunkUsage>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDelta>,
}

#[derive(Deserialize)]
struct ToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FunctionDelta>,
}

#[derive(Deserialize, Default)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct ChunkUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: u64,
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

/// Folds Chat Completions chunks into model events.
#[derive(Default)]
struct Decoder {
    calls: Vec<PartialCall>,
    stop: Option<StopReason>,
}

impl Decoder {
    fn feed(&mut self, data: &str) -> Result<Vec<ModelEvent>, ModelError> {
        let chunk: Chunk =
            serde_json::from_str(data).map_err(|e| ModelError::Decode(e.to_string()))?;
        let mut out = Vec::new();
        if let Some(usage) = chunk.usage {
            out.push(ModelEvent::Usage {
                usage: Usage {
                    input_tokens: usage.prompt_tokens,
                    output_tokens: usage.completion_tokens,
                    cached_input_tokens: usage.prompt_tokens_details.map_or(0, |d| d.cached_tokens),
                },
            });
        }
        for choice in chunk.choices {
            self.choice(choice, &mut out);
        }
        Ok(out)
    }

    fn choice(&mut self, choice: Choice, out: &mut Vec<ModelEvent>) {
        if let Some(text) = choice.delta.content
            && !text.is_empty()
        {
            out.push(ModelEvent::TextDelta { text });
        }
        for delta in choice.delta.tool_calls {
            while self.calls.len() <= delta.index {
                self.calls.push(PartialCall::default());
            }
            let call = &mut self.calls[delta.index];
            if let Some(id) = delta.id {
                call.id = id;
            }
            if let Some(function) = delta.function {
                if let Some(name) = function.name {
                    call.name.push_str(&name);
                }
                if let Some(arguments) = function.arguments {
                    call.arguments.push_str(&arguments);
                }
            }
        }
        if let Some(reason) = choice.finish_reason {
            self.stop = Some(match reason.as_str() {
                "stop" => StopReason::EndTurn,
                "tool_calls" => StopReason::ToolUse,
                "length" => StopReason::MaxTokens,
                other => StopReason::Other(other.to_owned()),
            });
        }
    }

    fn finish(&mut self) -> Vec<ModelEvent> {
        let mut out: Vec<ModelEvent> = self
            .calls
            .drain(..)
            .filter(|c| !c.name.is_empty())
            .map(|c| ModelEvent::ToolCall {
                call: finish_call(c),
            })
            .collect();
        out.push(ModelEvent::Stop {
            reason: self.stop.take().unwrap_or(StopReason::EndTurn),
        });
        out
    }
}

/// Turns Chat Completions SSE chunks into model events.
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

fn finish_call(call: PartialCall) -> ToolCall {
    let arguments =
        serde_json::from_str(&call.arguments).unwrap_or(Value::Object(serde_json::Map::new()));
    ToolCall {
        id: call.id,
        name: call.name,
        arguments,
    }
}

harness_async_trait! {
    impl Llm for OpenAiLlm {
        fn capabilities(&self) -> &ModelCapabilities {
            &self.caps
        }

        async fn stream(&self, request: ModelRequest) -> Result<EventStream<'static>, ModelError> {
            check_modalities(&self.caps, &request)?;
            let mut http_request = HttpRequest::new(http::Method::POST, self.endpoint());
            http_request.headers.insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("application/json"));
            let auth = http::HeaderValue::from_str(&format!("Bearer {}", self.api_key.expose()))
                .map_err(|_| ModelError::NonRetryable { status: None, message: "credential is not a valid header value".into() })?;
            http_request.headers.insert(http::header::AUTHORIZATION, auth);
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
            Ok(WireNames::new(&request.tools).restore(decode(response.body)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenAiLlm, decode};
    use crate::assets::{DataRole, Prompt};
    use crate::harness::models::llm::sse::chunks;
    use crate::harness::models::llm::{
        ContextMessage, Effort, Llm, MessageRole, ModelError, ModelRequest, StopReason, collect,
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
                ContextMessage::tool_result("call_1", "42"),
            ],
        );
        req.messages[1]
            .tool_calls
            .push(crate::harness::tools::ToolCall {
                id: "call_1".into(),
                name: "add".into(),
                arguments: serde_json::json!({"a": 1}),
            });
        req.tools.push(ToolDescriptor {
            name: "add".into(),
            description: "adds".into(),
            parameters: serde_json::json!({"type": "object"}),
            idempotency: Idempotency::Idempotent,
        });
        req.effort = Some(Effort::High);
        req.max_output_tokens = Some(256);
        req
    }

    fn client(http: Arc<MockHttp>) -> OpenAiLlm {
        OpenAiLlm::new(
            "default",
            "gpt-6.1",
            "https://api.openai.com/v1".parse().unwrap(),
            Secret::new("sk-canary-openai".into()),
            http,
        )
    }

    #[test]
    fn snapshot_openai_request_body() {
        let body = client(Arc::new(MockHttp::always(http::StatusCode::OK, ""))).body(&request());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/snapshots/openai_request_body.json");
        let json = serde_json::to_string_pretty(&body).unwrap() + "\n";
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            std::fs::write(&path, &json).unwrap();
        }
        let published: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default())
                .expect("snapshot is JSON");
        assert_eq!(
            published, body,
            "openai_request_body.json is stale: rerun with UPDATE_SNAPSHOTS=1"
        );
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["reasoning_effort"], "high");
    }

    #[tokio::test]
    async fn streams_text_tool_calls_and_usage_and_sets_the_bearer_header() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let http = Arc::new(MockHttp::new(move |req| {
            seen2.lock().unwrap().push(
                req.headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_owned(),
            );
            Ok(HttpResponse { status: http::StatusCode::OK, headers: http::HeaderMap::new(), body: concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_9\",\"function\":{\"name\":\"add\",\"arguments\":\"{\\\"a\\\":\"}}]}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"1}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":4,\"prompt_tokens_details\":{\"cached_tokens\":8}}}\n\n",
                "data: [DONE]\n\n").into() })
        }));
        let llm = client(http);
        let turn = collect(llm.stream(request()).await.expect("stream"), None)
            .await
            .expect("turn");
        assert_eq!(turn.text, "Hello");
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "add");
        assert_eq!(turn.tool_calls[0].arguments, serde_json::json!({"a": 1}));
        assert_eq!(turn.stop, Some(StopReason::ToolUse));
        assert_eq!(turn.usage.map(|u| u.cached_input_tokens), Some(8));
        assert_eq!(seen.lock().unwrap().as_slice(), ["Bearer sk-canary-openai"]);
    }

    #[tokio::test]
    async fn dotted_tool_names_go_out_sanitised_and_come_back_as_ours() {
        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let bodies2 = bodies.clone();
        let http = Arc::new(MockHttp::new(move |req| {
            bodies2
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&req.body).into_owned());
            Ok(HttpResponse { status: http::StatusCode::OK, headers: http::HeaderMap::new(), body: concat!(
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"agent_call\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
                "data: [DONE]\n\n").into() })
        }));
        let mut req = request();
        req.tools[0].name = "agent.call".into();
        req.messages[1].tool_calls[0].name = "agent.call".into();
        let turn = collect(client(http).stream(req).await.expect("stream"), None)
            .await
            .expect("turn");
        assert_eq!(turn.tool_calls[0].name, "agent.call");
        let body = bodies.lock().unwrap()[0].clone();
        assert!(body.contains("\"name\":\"agent_call\""), "{body}");
        assert!(
            !body.contains("agent.call"),
            "no dotted name reaches the provider: {body}"
        );
    }

    #[tokio::test]
    async fn statuses_classify_into_retryable_and_not() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::RETRY_AFTER,
            http::HeaderValue::from_static("7"),
        );
        let http = Arc::new(MockHttp::new(move |_| {
            Ok(HttpResponse {
                status: http::StatusCode::TOO_MANY_REQUESTS,
                headers: headers.clone(),
                body: "slow down".into(),
            })
        }));
        let Err(err) = client(http).stream(request()).await else {
            panic!("429 must fail")
        };
        assert!(
            matches!(err, ModelError::Retryable { status: Some(429), retry_after: Some(d), .. } if d.as_secs() == 7)
        );
        let http = Arc::new(MockHttp::always(http::StatusCode::UNAUTHORIZED, "bad key"));
        assert!(matches!(
            client(http).stream(request()).await,
            Err(ModelError::NonRetryable {
                status: Some(401),
                ..
            })
        ));
        let bad: Vec<_> =
            futures::StreamExt::collect::<Vec<_>>(decode(chunks(vec!["data: not json\n\n"]))).await;
        assert!(matches!(bad[0], Err(ModelError::Decode(_))));
    }
}

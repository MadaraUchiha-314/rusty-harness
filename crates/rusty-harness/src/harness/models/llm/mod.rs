//! The provider-agnostic `Llm` interface and the `OpenAI` and Anthropic thin clients
//! (R9.1–3).
//!
//! A request is a [`Prompt`] for the instructions plus typed messages, tools and cache
//! hints; a response is a stream of [`ModelEvent`]s (text deltas, tool-call deltas,
//! usage, the stop reason) that [`collect`] folds into one [`ModelTurn`]. Retries with
//! exponential backoff wrap any `Llm` through [`Retrying`].

use core::fmt;
use core::time::Duration;
use std::collections::BTreeSet;
use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::assets::Prompt;
use crate::config::RetryConfig;
use crate::harness::tools::{ToolCall, ToolDescriptor};
use crate::harness_async_trait;
use crate::registry::{Entity, EntityId, EntityKind};
use crate::runtime::{HarnessStream, Runtime};

pub mod anthropic;
pub mod openai;
pub mod sse;
pub mod testing;

/// What a model can take in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Modality {
    /// Text.
    Text,
    /// Images.
    Image,
    /// Audio.
    Audio,
}

/// Reasoning effort (R9.5, R10.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Effort {
    /// Low.
    Low,
    /// Medium.
    Medium,
    /// High.
    High,
}

impl core::str::FromStr for Effort {
    type Err = ModelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            other => Err(ModelError::NonRetryable {
                status: None,
                message: format!("unknown effort {other:?}"),
            }),
        }
    }
}

impl fmt::Display for Effort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        })
    }
}

/// What a model supports (R9.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    /// Modalities it accepts as input.
    pub input_modalities: BTreeSet<Modality>,
    /// Tool calling.
    pub tools: bool,
    /// Provider-side prompt caching.
    pub caching: bool,
    /// The context window, in tokens.
    pub max_window_tokens: u32,
    /// Effort levels the provider honours.
    pub effort_levels: Vec<Effort>,
}

/// Who said a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MessageRole {
    /// A participant.
    User,
    /// The model.
    Assistant,
    /// A tool's result, answering one of the assistant's calls.
    Tool,
}

/// One piece of a message (R28.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum ContentPart {
    /// Text.
    Text {
        /// The text.
        text: String,
    },
    /// An image, inline.
    Image {
        /// `image/png`, `image/jpeg`, …
        media_type: String,
        /// The bytes.
        data: Bytes,
    },
    /// Audio, inline.
    Audio {
        /// `audio/wav`, `audio/mp3`, …
        media_type: String,
        /// The bytes.
        data: Bytes,
    },
}

/// One message of the conversation the model sees.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextMessage {
    /// The role.
    pub role: MessageRole,
    /// The content.
    pub parts: Vec<ContentPart>,
    /// For an assistant message: the tool calls it made.
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// For a tool message: the call it answers.
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

impl ContextMessage {
    /// A text message.
    #[must_use]
    pub fn text(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            role,
            parts: vec![ContentPart::Text { text: text.into() }],
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// A tool result answering `call_id`.
    #[must_use]
    pub fn tool_result(call_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Tool,
            parts: vec![ContentPart::Text { text: text.into() }],
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
        }
    }

    /// The modalities this message carries.
    #[must_use]
    pub fn modalities(&self) -> BTreeSet<Modality> {
        self.parts
            .iter()
            .map(|p| match p {
                ContentPart::Text { .. } => Modality::Text,
                ContentPart::Image { .. } => Modality::Image,
                ContentPart::Audio { .. } => Modality::Audio,
            })
            .collect()
    }
}

/// Where the stable prefix ends: the provider's cache may stop there (R15.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheBreakpoint {
    /// After the instructions (index 0) or after message `n` (index `n + 1`).
    pub after: usize,
}

/// One model call (R9.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    /// The model alias the call targets, for observability (R24.3); the router picks
    /// the client regardless.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The instructions: fragments and framed data only.
    pub instructions: Prompt,
    /// The conversation.
    pub messages: Vec<ContextMessage>,
    /// The tools the model may call.
    pub tools: Vec<ToolDescriptor>,
    /// Reasoning effort, when the provider honours it.
    pub effort: Option<Effort>,
    /// Cache breakpoints, for providers with explicit caching.
    pub cache_hints: Vec<CacheBreakpoint>,
    /// A cap on the answer.
    pub max_output_tokens: Option<u32>,
}

impl ModelRequest {
    /// A request with instructions and messages, no tools.
    #[must_use]
    pub fn new(instructions: Prompt, messages: Vec<ContextMessage>) -> Self {
        Self {
            model: None,
            instructions,
            messages,
            tools: Vec::new(),
            effort: None,
            cache_hints: Vec::new(),
            max_output_tokens: None,
        }
    }

    /// The modalities the messages carry.
    #[must_use]
    pub fn modalities(&self) -> BTreeSet<Modality> {
        self.messages
            .iter()
            .flat_map(ContextMessage::modalities)
            .collect()
    }
}

/// Token usage of one call (R9.7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Prompt tokens.
    pub input_tokens: u64,
    /// Completion tokens.
    pub output_tokens: u64,
    /// Prompt tokens served from the provider's cache.
    pub cached_input_tokens: u64,
}

/// Why the model stopped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopReason {
    /// It finished its answer.
    EndTurn,
    /// It wants tools run.
    ToolUse,
    /// It hit the output cap.
    MaxTokens,
    /// Something provider-specific.
    Other(String),
}

/// One event of a streamed answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "event")]
pub enum ModelEvent {
    /// More answer text.
    TextDelta {
        /// The text.
        text: String,
    },
    /// A tool call, complete.
    ToolCall {
        /// The call.
        call: ToolCall,
    },
    /// Usage, usually once at the end.
    Usage {
        /// The counts.
        usage: Usage,
    },
    /// The model stopped.
    Stop {
        /// Why.
        reason: StopReason,
    },
}

/// A whole answer, folded from its events.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTurn {
    /// The answer text.
    pub text: String,
    /// The tool calls, in the order the model issued them.
    pub tool_calls: Vec<ToolCall>,
    /// Usage, when the provider reported it.
    pub usage: Option<Usage>,
    /// Why it stopped, when the provider said.
    pub stop: Option<StopReason>,
    /// Wall-clock latency of the call.
    pub latency: Duration,
}

/// A model call failure (R9.6).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ModelError {
    /// Worth retrying: rate limits, overload, 5xx, transport.
    #[error("retryable model error ({status:?}): {message}")]
    Retryable {
        /// The HTTP status, when there was a response.
        status: Option<u16>,
        /// What the provider said.
        message: String,
        /// A wait the provider asked for.
        retry_after: Option<Duration>,
    },
    /// Not worth retrying: bad request, authentication, an unknown model.
    #[error("model error ({status:?}): {message}")]
    NonRetryable {
        /// The HTTP status, when there was a response.
        status: Option<u16>,
        /// What the provider said.
        message: String,
    },
    /// The request carries a modality this model does not take (R28.1).
    #[error("this model does not accept {0:?} input")]
    UnsupportedModality(Modality),
    /// The provider's response could not be decoded.
    #[error("could not decode the model's response: {0}")]
    Decode(String),
}

impl ModelError {
    /// Whether a retry under the policy makes sense.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable { .. })
    }

    /// Classifies an HTTP status.
    #[must_use]
    pub fn from_status(status: u16, message: String, retry_after: Option<Duration>) -> Self {
        if status == 408 || status == 409 || status == 429 || status >= 500 {
            Self::Retryable {
                status: Some(status),
                message,
                retry_after,
            }
        } else {
            Self::NonRetryable {
                status: Some(status),
                message,
            }
        }
    }
}

/// A stream of model events.
pub type EventStream<'a> = HarnessStream<'a, Result<ModelEvent, ModelError>>;

harness_async_trait! {
    /// A language model (R9.1).
    pub trait Llm: Entity {
        /// What it supports.
        fn capabilities(&self) -> &ModelCapabilities;
        /// Streams the answer to `request`; a `NonRetryable` error for a modality the
        /// model lacks comes before any event.
        async fn stream(&self, request: ModelRequest) -> Result<EventStream<'static>, ModelError>;
    }
}

/// Tool names as the providers accept them: `OpenAI` and Anthropic both require
/// `^[a-zA-Z0-9_-]{1,64}$`, and the harness's own names carry dots (`agent.call`,
/// `server.tool`). Built per request from its tools; anything else maps through the same
/// rule so a tool call in the history still matches.
#[derive(Debug, Default)]
pub struct WireNames {
    to_wire: std::collections::BTreeMap<String, String>,
    from_wire: std::collections::BTreeMap<String, String>,
}

impl WireNames {
    /// The longest name the providers take.
    pub const MAX: usize = 64;

    /// One wire name per tool of `tools`, unique even when two sanitise alike.
    #[must_use]
    pub fn new(tools: &[ToolDescriptor]) -> Self {
        let mut names = Self::default();
        for tool in tools {
            let base = Self::sanitise(&tool.name);
            let mut wire = base.clone();
            let mut n = 2;
            while names.from_wire.contains_key(&wire) {
                let suffix = format!("_{n}");
                let keep = Self::MAX.saturating_sub(suffix.len()).min(base.len());
                wire = format!("{}{suffix}", &base[..keep]);
                n += 1;
            }
            names.to_wire.insert(tool.name.clone(), wire.clone());
            names.from_wire.insert(wire, tool.name.clone());
        }
        names
    }

    /// The name to send for `ours`.
    #[must_use]
    pub fn wire(&self, ours: &str) -> String {
        self.to_wire
            .get(ours)
            .cloned()
            .unwrap_or_else(|| Self::sanitise(ours))
    }

    /// The harness's name for a `wire` name the model used.
    #[must_use]
    pub fn ours(&self, wire: &str) -> String {
        self.from_wire
            .get(wire)
            .cloned()
            .unwrap_or_else(|| wire.to_owned())
    }

    /// Maps the tool calls of a decoded stream back to the harness's names.
    pub fn restore(self, events: EventStream<'static>) -> EventStream<'static> {
        crate::runtime::boxed_stream(events.map(move |event| match event {
            Ok(ModelEvent::ToolCall { mut call }) => {
                call.name = self.ours(&call.name);
                Ok(ModelEvent::ToolCall { call })
            }
            other => other,
        }))
    }

    fn sanitise(name: &str) -> String {
        let mut out: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if out.is_empty() {
            out.push('_');
        }
        out.truncate(Self::MAX);
        out
    }
}

/// Checks the request's modalities against the model's.
///
/// # Errors
///
/// [`ModelError::UnsupportedModality`] naming the first unsupported modality.
pub fn check_modalities(
    caps: &ModelCapabilities,
    request: &ModelRequest,
) -> Result<(), ModelError> {
    request
        .modalities()
        .into_iter()
        .find(|m| !caps.input_modalities.contains(m))
        .map_or(Ok(()), |m| Err(ModelError::UnsupportedModality(m)))
}

/// Folds a stream into one turn, measuring latency from `started`.
///
/// # Errors
///
/// The first error of the stream.
pub async fn collect(
    mut events: EventStream<'_>,
    started: Option<crate::runtime::Instant>,
) -> Result<ModelTurn, ModelError> {
    let mut turn = ModelTurn::default();
    while let Some(event) = events.next().await {
        match event? {
            ModelEvent::TextDelta { text } => turn.text.push_str(&text),
            ModelEvent::ToolCall { call } => turn.tool_calls.push(call),
            ModelEvent::Usage { usage } => turn.usage = Some(usage),
            ModelEvent::Stop { reason } => turn.stop = Some(reason),
        }
    }
    turn.latency = started.map(|s| s.elapsed()).unwrap_or_default();
    if let Some(usage) = turn.usage {
        tracing::info!(
            input_tokens = usage.input_tokens,
            output_tokens = usage.output_tokens,
            cached_input_tokens = usage.cached_input_tokens,
            latency_ms = u64::try_from(turn.latency.as_millis()).unwrap_or(u64::MAX),
            "model turn"
        );
    }
    Ok(turn)
}

/// Retries another model's calls under a [`RetryConfig`] (R9.6).
pub struct Retrying<L, R> {
    inner: Arc<L>,
    runtime: R,
    policy: RetryConfig,
}

impl<L: fmt::Debug, R> fmt::Debug for Retrying<L, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Retrying")
            .field("inner", &self.inner)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl<L: Llm, R: Runtime + Send + Sync + 'static> Retrying<L, R> {
    /// Wraps `inner`.
    #[must_use]
    pub fn new(inner: Arc<L>, runtime: R, policy: RetryConfig) -> Self {
        Self {
            inner,
            runtime,
            policy,
        }
    }

    /// The wait before attempt `n` (1-based).
    #[must_use]
    pub fn backoff(&self, attempt: u32) -> Duration {
        let initial: Duration = self.policy.initial.into();
        let max: Duration = self.policy.max_interval.into();
        let factor = self
            .policy
            .coefficient
            .saturating_pow(attempt.saturating_sub(1));
        initial.saturating_mul(factor).min(max)
    }
}

impl<L: Llm, R: Runtime + Send + Sync + 'static> Entity for Retrying<L, R> {
    fn kind(&self) -> EntityKind {
        EntityKind::Llm
    }
    fn entity_id(&self) -> &EntityId {
        self.inner.entity_id()
    }
}

harness_async_trait! {
    impl<L: Llm, R: Runtime + Send + Sync + 'static> Llm for Retrying<L, R> {
        fn capabilities(&self) -> &ModelCapabilities {
            self.inner.capabilities()
        }

        async fn stream(&self, request: ModelRequest) -> Result<EventStream<'static>, ModelError> {
            let mut attempt = 1;
            loop {
                match self.inner.stream(request.clone()).await {
                    Ok(stream) => return Ok(stream),
                    Err(err) if err.is_retryable() && attempt < self.policy.max_attempts => {
                        let wait = match &err {
                            ModelError::Retryable { retry_after: Some(after), .. } => *after,
                            _ => self.backoff(attempt),
                        };
                        tracing::warn!(attempt, wait_ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX), error = %err, "model call failed; retrying");
                        self.runtime.sleep(wait).await;
                        attempt += 1;
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }
}

/// Parses a `Retry-After` header value (seconds).
#[must_use]
pub fn retry_after(headers: &http::HeaderMap) -> Option<Duration> {
    headers
        .get(http::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::{
        ContextMessage, Effort, Llm, MessageRole, Modality, ModelCapabilities, ModelError,
        ModelEvent, ModelRequest, Retrying, StopReason, Usage, check_modalities, collect,
    };
    use crate::assets::{DataRole, Prompt};
    use crate::config::{DurationStr, RetryConfig};
    use crate::harness::models::llm::testing::RecordedLlm;
    use crate::runtime::TokioRuntime;
    use bytes::Bytes;
    use std::collections::BTreeSet;
    use std::sync::Arc;

    fn text_only() -> ModelCapabilities {
        ModelCapabilities {
            input_modalities: BTreeSet::from([Modality::Text]),
            tools: true,
            caching: false,
            max_window_tokens: 1000,
            effort_levels: vec![],
        }
    }

    #[test]
    fn unsupported_modality_is_a_typed_error() {
        let err = ModelError::UnsupportedModality(Modality::Audio);
        assert!(
            !err.is_retryable(),
            "a modality mismatch never heals by retrying"
        );
        assert!(err.to_string().contains("Audio"), "{err}");
        let mut req = ModelRequest::new(
            Prompt::data(DataRole::TaskField, "x"),
            vec![ContextMessage::text(MessageRole::User, "hi")],
        );
        req.messages.push(ContextMessage {
            role: MessageRole::User,
            parts: vec![super::ContentPart::Audio {
                media_type: "audio/wav".into(),
                data: Bytes::from_static(b"RIFF"),
            }],
            tool_calls: vec![],
            tool_call_id: None,
        });
        assert_eq!(check_modalities(&text_only(), &req), Err(err));
    }

    #[test]
    fn a_request_with_an_image_is_refused_by_a_text_only_model_naming_the_modality() {
        let mut req = ModelRequest::new(
            Prompt::data(DataRole::TaskField, "x"),
            vec![ContextMessage::text(MessageRole::User, "hi")],
        );
        assert!(check_modalities(&text_only(), &req).is_ok());
        req.messages.push(ContextMessage {
            role: MessageRole::User,
            parts: vec![super::ContentPart::Image {
                media_type: "image/png".into(),
                data: Bytes::from_static(b"png"),
            }],
            tool_calls: vec![],
            tool_call_id: None,
        });
        assert_eq!(
            check_modalities(&text_only(), &req),
            Err(ModelError::UnsupportedModality(Modality::Image))
        );
        assert_eq!("high".parse::<Effort>().unwrap(), Effort::High);
        assert!("extreme".parse::<Effort>().is_err());
    }

    #[tokio::test]
    async fn collect_folds_events_into_a_turn() {
        let llm = RecordedLlm::new(
            "rec",
            vec![vec![
                ModelEvent::TextDelta { text: "Hel".into() },
                ModelEvent::TextDelta { text: "lo".into() },
                ModelEvent::Usage {
                    usage: Usage {
                        input_tokens: 3,
                        output_tokens: 2,
                        cached_input_tokens: 0,
                    },
                },
                ModelEvent::Stop {
                    reason: StopReason::EndTurn,
                },
            ]],
        );
        let req = ModelRequest::new(Prompt::data(DataRole::TaskField, "x"), vec![]);
        let turn = collect(
            llm.stream(req).await.expect("stream"),
            Some(crate::runtime::Instant::now()),
        )
        .await
        .expect("turn");
        assert_eq!(turn.text, "Hello");
        assert_eq!(turn.usage.map(|u| u.output_tokens), Some(2));
        assert_eq!(turn.stop, Some(StopReason::EndTurn));
    }

    #[tokio::test(start_paused = true)]
    async fn retryable_errors_are_retried_with_backoff_and_non_retryable_are_not() {
        let flaky = RecordedLlm::failing_then(
            "flaky",
            2,
            true,
            vec![ModelEvent::Stop {
                reason: StopReason::EndTurn,
            }],
        );
        let policy = RetryConfig {
            initial: DurationStr::secs(1),
            coefficient: 2,
            max_interval: DurationStr::secs(100),
            max_attempts: 5,
        };
        let retrying = Retrying::new(Arc::new(flaky), TokioRuntime::current(), policy.clone());
        assert_eq!(retrying.backoff(1).as_secs(), 1);
        assert_eq!(retrying.backoff(3).as_secs(), 4);
        assert_eq!(
            retrying.backoff(20).as_secs(),
            100,
            "capped at max_interval"
        );
        let req = ModelRequest::new(Prompt::data(DataRole::TaskField, "x"), vec![]);
        let started = tokio::time::Instant::now();
        let turn = collect(
            retrying
                .stream(req.clone())
                .await
                .expect("third attempt succeeds"),
            None,
        )
        .await
        .expect("turn");
        assert_eq!(turn.stop, Some(StopReason::EndTurn));
        assert_eq!(started.elapsed().as_secs(), 3, "waited 1s then 2s");
        assert_eq!(retrying.inner.attempts(), 3);

        let fatal = RecordedLlm::failing_then("fatal", 1, false, vec![]);
        let retrying = Retrying::new(Arc::new(fatal), TokioRuntime::current(), policy);
        assert!(matches!(
            retrying.stream(req).await,
            Err(ModelError::NonRetryable { .. })
        ));
        assert_eq!(retrying.inner.attempts(), 1);
    }

    #[test]
    fn tool_names_go_out_as_the_providers_accept_and_come_back_as_ours() {
        use super::WireNames;
        use crate::harness::tools::ToolDescriptor;
        let tool = |name: &str| ToolDescriptor {
            name: name.into(),
            description: String::new(),
            parameters: serde_json::json!({ "type": "object" }),
            idempotency: crate::harness::tools::Idempotency::Idempotent,
        };
        let long = "x.".repeat(40);
        let names = WireNames::new(&[
            tool("agent.call"),
            tool("agent_call"),
            tool("ok-name"),
            tool(&long),
        ]);
        assert_eq!(names.wire("agent.call"), "agent_call");
        assert_eq!(
            names.wire("agent_call"),
            "agent_call_2",
            "a collision gets a suffix"
        );
        assert_eq!(names.wire("ok-name"), "ok-name");
        assert_eq!(names.wire(&long).len(), WireNames::MAX);
        for ours in ["agent.call", "agent_call", "ok-name", long.as_str()] {
            let wire = names.wire(ours);
            assert!(
                wire.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "{wire}"
            );
            assert_eq!(names.ours(&wire), ours);
        }
        assert_eq!(
            names.wire("gone.tool"),
            "gone_tool",
            "a tool no longer offered still maps"
        );
    }
}

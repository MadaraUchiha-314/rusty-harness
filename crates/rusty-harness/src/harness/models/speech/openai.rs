//! The `OpenAI` speech providers (R28.3–4, Q15): transcription and speech-to-speech over the
//! Realtime `WebSocket`, synthesis over `/v1/audio/speech`.
//!
//! Every connection is opened by the harness with the key from the secret resolver; the
//! key rides on the `Authorization` header of the `WebSocket` handshake or the HTTP request
//! and nowhere else, so a surface streaming audio never holds it (boundary 6, abuse 11).
//! Audio is 16-bit PCM at 24 kHz, mono, both ways — the format the Realtime API speaks.

use std::sync::Arc;

use base64::Engine as _;
use bytes::Bytes;
use futures::StreamExt as _;
use serde_json::{Value, json};

use super::{
    AudioChunk, SpeechToText, TextToSpeech, TranscriptEvent, Voice, VoiceEvent, VoiceId,
    VoiceSession, VoiceSessionConfig,
};
use crate::config::VoiceConfig;
use crate::harness::models::llm::ModelError;
use crate::harness::tools::ToolCall;
use crate::harness_async_trait;
use crate::host::{
    AudioFormat, AudioStream, Host, HostError, HttpClient, HttpRequest, Secret, WebSocket,
    WebSocketClient, WsMessage,
};
use crate::registry::{Entity, EntityId, EntityKind};
use crate::runtime::{HarnessStream, boxed_stream};

/// The audio both directions use: PCM16 at 24 kHz, mono.
pub const PCM24K: AudioFormat = AudioFormat::Pcm16 {
    sample_rate: 24_000,
    channels: 1,
};

/// The `OpenAI` API base.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
/// The speech-to-speech model when `[voice] model` is absent.
pub const DEFAULT_REALTIME_MODEL: &str = "gpt-realtime-2.1";
/// The streaming transcription model when `[voice] transcribe_model` is absent.
pub const DEFAULT_TRANSCRIBE_MODEL: &str = "gpt-live-transcribe";
/// The synthesis model when `[voice] speech_model` is absent.
pub const DEFAULT_SPEECH_MODEL: &str = "gpt-4o-mini-tts";
/// The voice when `[voice] voice` is absent.
pub const DEFAULT_VOICE: &str = "marin";

/// What the three providers share: where, which models, and the key.
pub struct OpenAiSpeechClient {
    base_url: url::Url,
    realtime_model: String,
    transcribe_model: String,
    speech_model: String,
    api_key: Secret<String>,
    http: Arc<dyn HttpClient>,
    websocket: Arc<dyn WebSocketClient>,
}

impl core::fmt::Debug for OpenAiSpeechClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpenAiSpeechClient")
            .field("base_url", &self.base_url.as_str())
            .field("realtime_model", &self.realtime_model)
            .finish_non_exhaustive()
    }
}

fn host_error(e: &HostError) -> ModelError {
    match e {
        HostError::Http {
            status: Some(status),
            message,
        } => ModelError::from_status(*status, message.clone(), None),
        HostError::Timeout => ModelError::Retryable {
            status: None,
            message: "timeout".into(),
            retry_after: None,
        },
        other => ModelError::Retryable {
            status: None,
            message: other.to_string(),
            retry_after: None,
        },
    }
}

fn fatal(message: impl Into<String>) -> ModelError {
    ModelError::NonRetryable {
        status: None,
        message: message.into(),
    }
}

fn require_pcm24k(format: AudioFormat) -> Result<(), ModelError> {
    if format == PCM24K {
        Ok(())
    } else {
        Err(fatal(format!(
            "the OpenAI speech session takes 16-bit PCM at 24 kHz mono, not {format:?}"
        )))
    }
}

impl OpenAiSpeechClient {
    /// A client for `base_url` (`…/v1`) with `api_key`.
    #[must_use]
    pub fn new(
        base_url: url::Url,
        api_key: Secret<String>,
        http: Arc<dyn HttpClient>,
        websocket: Arc<dyn WebSocketClient>,
    ) -> Self {
        Self {
            base_url,
            realtime_model: DEFAULT_REALTIME_MODEL.into(),
            transcribe_model: DEFAULT_TRANSCRIBE_MODEL.into(),
            speech_model: DEFAULT_SPEECH_MODEL.into(),
            api_key,
            http,
            websocket,
        }
    }

    /// The client `[voice]` describes, the key resolved through the host.
    ///
    /// # Errors
    ///
    /// A missing key reference, a reference that does not resolve, or a host without
    /// HTTP or `WebSocket`s.
    pub async fn from_config(config: &VoiceConfig, host: &Host) -> Result<Self, ModelError> {
        let reference = config
            .api_key
            .as_ref()
            .ok_or_else(|| fatal("[voice] api_key is required"))?;
        let api_key = host
            .secrets()
            .map_err(|e| fatal(e.to_string()))?
            .resolve(reference)
            .await
            .map_err(|e| fatal(e.to_string()))?;
        let http = host.http().map_err(|e| fatal(e.to_string()))?;
        let websocket = host.websocket().map_err(|e| fatal(e.to_string()))?;
        let base_url = match &config.base_url {
            Some(url) => url.clone(),
            None => DEFAULT_BASE_URL
                .parse()
                .map_err(|_| ModelError::Decode("default base url".into()))?,
        };
        let mut client = Self::new(base_url, api_key, http, websocket);
        if let Some(m) = &config.model {
            client.realtime_model.clone_from(m);
        }
        if let Some(m) = &config.transcribe_model {
            client.transcribe_model.clone_from(m);
        }
        if let Some(m) = &config.speech_model {
            client.speech_model.clone_from(m);
        }
        Ok(client)
    }

    fn auth(&self) -> Result<http::HeaderMap, ModelError> {
        let mut headers = http::HeaderMap::new();
        let mut value = http::HeaderValue::from_str(&format!("Bearer {}", self.api_key.expose()))
            .map_err(|_| fatal("the voice key is not a valid header value"))?;
        value.set_sensitive(true);
        headers.insert(http::header::AUTHORIZATION, value);
        Ok(headers)
    }

    /// `wss://…/v1/realtime?<query>`.
    fn realtime_url(&self, query: &[(&str, &str)]) -> Result<url::Url, ModelError> {
        let mut url = self.base_url.clone();
        let scheme = if url.scheme() == "http" { "ws" } else { "wss" };
        url.set_scheme(scheme)
            .map_err(|()| fatal("the voice base url cannot carry a websocket"))?;
        let path = format!("{}/realtime", url.path().trim_end_matches('/'));
        url.set_path(&path);
        url.query_pairs_mut().clear().extend_pairs(query);
        Ok(url)
    }

    async fn open(&self, url: url::Url, session: Value) -> Result<Box<dyn WebSocket>, ModelError> {
        let mut socket = self
            .websocket
            .connect(url, self.auth()?)
            .await
            .map_err(|e| host_error(&e))?;
        send_json(
            socket.as_mut(),
            &json!({ "type": "session.update", "session": session }),
        )
        .await?;
        Ok(socket)
    }
}

async fn send_json(socket: &mut dyn WebSocket, value: &Value) -> Result<(), ModelError> {
    socket
        .send(WsMessage::Text(value.to_string()))
        .await
        .map_err(|e| host_error(&e))
}

/// The `session.update` body of a speech-to-speech session (snapshotted:
/// `snapshot_realtime_session_config`).
#[must_use]
pub fn realtime_session_config(
    model: &str,
    transcribe_model: &str,
    config: &VoiceSessionConfig,
) -> Value {
    let tools: Vec<Value> = config
        .tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            })
        })
        .collect();
    json!({
        "type": "realtime",
        "model": model,
        "instructions": config.instructions,
        "output_modalities": ["audio"],
        "audio": {
            "input": {
                "format": { "type": "audio/pcm", "rate": 24_000 },
                "turn_detection": { "type": "semantic_vad" },
                "transcription": { "model": transcribe_model },
            },
            "output": {
                "format": { "type": "audio/pcm", "rate": 24_000 },
                "voice": config.voice.0,
            },
        },
        "tools": tools,
        "tool_choice": "auto",
    })
}

/// The `session.update` body of a transcription session.
#[must_use]
pub fn transcription_session_config(model: &str) -> Value {
    json!({
        "type": "transcription",
        "audio": {
            "input": {
                "format": { "type": "audio/pcm", "rate": 24_000 },
                "transcription": { "model": model },
                "turn_detection": null,
            },
        },
    })
}

/// One server event of a Realtime session, as the harness reads it.
#[derive(Clone, Debug, PartialEq)]
pub enum RealtimeEvent {
    /// Model audio.
    Audio(Bytes),
    /// The transcript of the model's audio, a delta.
    Transcript(String),
    /// The transcript of the user's audio, a delta.
    UserTranscriptDelta(String),
    /// The transcript of the user's audio, complete.
    UserTranscript(String),
    /// A tool call, its arguments complete.
    ToolCall(ToolCall),
    /// A response finished.
    ResponseDone,
    /// An error the server reported.
    Error(String),
    /// Anything else (session acknowledgements, rate limits, speech markers).
    Other,
}

/// Parses one server frame.
///
/// # Errors
///
/// A frame that is not a JSON object with a `type`.
pub fn parse_realtime(frame: &str) -> Result<RealtimeEvent, ModelError> {
    let v: Value = serde_json::from_str(frame).map_err(|e| ModelError::Decode(e.to_string()))?;
    let kind = v["type"]
        .as_str()
        .ok_or_else(|| ModelError::Decode("a realtime event without a type".into()))?;
    let text = |key: &str| v[key].as_str().unwrap_or_default().to_owned();
    Ok(match kind {
        "response.output_audio.delta" => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(v["delta"].as_str().unwrap_or_default())
                .map_err(|e| ModelError::Decode(format!("audio delta: {e}")))?;
            RealtimeEvent::Audio(Bytes::from(bytes))
        }
        "response.output_audio_transcript.delta" => RealtimeEvent::Transcript(text("delta")),
        "conversation.item.input_audio_transcription.delta" => {
            RealtimeEvent::UserTranscriptDelta(text("delta"))
        }
        "conversation.item.input_audio_transcription.completed" => {
            RealtimeEvent::UserTranscript(text("transcript"))
        }
        "response.function_call_arguments.done" => RealtimeEvent::ToolCall(ToolCall {
            id: text("call_id"),
            name: text("name"),
            arguments: serde_json::from_str(v["arguments"].as_str().unwrap_or("{}"))
                .unwrap_or(Value::Object(serde_json::Map::new())),
        }),
        "response.done" => RealtimeEvent::ResponseDone,
        "error" => RealtimeEvent::Error(
            v["error"]["message"]
                .as_str()
                .unwrap_or("the realtime session reported an error")
                .to_owned(),
        ),
        _ => RealtimeEvent::Other,
    })
}

/// The client frame appending user audio.
#[must_use]
pub fn audio_append(audio: &[u8]) -> Value {
    json!({
        "type": "input_audio_buffer.append",
        "audio": base64::engine::general_purpose::STANDARD.encode(audio),
    })
}

/// A speech-to-speech session over the Realtime `WebSocket`.
pub struct OpenAiVoiceSession {
    socket: Box<dyn WebSocket>,
    seq: u64,
    ended: bool,
}

impl core::fmt::Debug for OpenAiVoiceSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpenAiVoiceSession")
            .field("seq", &self.seq)
            .finish_non_exhaustive()
    }
}

harness_async_trait! {
    impl VoiceSession for OpenAiVoiceSession {
        async fn send_audio(&mut self, chunk: AudioChunk) -> Result<(), ModelError> {
            require_pcm24k(chunk.format)?;
            send_json(self.socket.as_mut(), &audio_append(&chunk.data)).await
        }

        async fn tool_result(&mut self, call_id: String, result: String) -> Result<(), ModelError> {
            send_json(
                self.socket.as_mut(),
                &json!({
                    "type": "conversation.item.create",
                    "item": { "type": "function_call_output", "call_id": call_id, "output": result },
                }),
            )
            .await?;
            send_json(self.socket.as_mut(), &json!({ "type": "response.create" })).await
        }

        async fn next(&mut self) -> Option<Result<VoiceEvent, ModelError>> {
            loop {
                if self.ended {
                    return None;
                }
                let frame = match self.socket.next().await {
                    None | Some(Ok(WsMessage::Close)) => {
                        self.ended = true;
                        return Some(Ok(VoiceEvent::Ended));
                    }
                    Some(Err(e)) => return Some(Err(host_error(&e))),
                    Some(Ok(WsMessage::Text(t))) => t,
                    Some(Ok(_)) => continue,
                };
                let event = match parse_realtime(&frame) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                return Some(Ok(match event {
                    RealtimeEvent::Audio(data) => {
                        self.seq += 1;
                        VoiceEvent::Audio(AudioChunk { format: PCM24K, data, seq: self.seq })
                    }
                    RealtimeEvent::Transcript(t) => VoiceEvent::Transcript(t),
                    RealtimeEvent::UserTranscript(t) => VoiceEvent::UserTranscript(t),
                    RealtimeEvent::ToolCall(call) => VoiceEvent::ToolCall(call),
                    RealtimeEvent::Error(message) => return Some(Err(fatal(message))),
                    RealtimeEvent::UserTranscriptDelta(_)
                    | RealtimeEvent::ResponseDone
                    | RealtimeEvent::Other => continue,
                }));
            }
        }

        async fn close(&mut self) -> Result<(), ModelError> {
            self.ended = true;
            self.socket.close().await.map_err(|e| host_error(&e))
        }
    }
}

macro_rules! provider {
    ($name:ident, $kind:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone)]
        pub struct $name {
            id: EntityId,
            client: Arc<OpenAiSpeechClient>,
        }

        impl $name {
            /// The provider over `client`, registered as `id`.
            #[must_use]
            pub fn new(id: &str, client: Arc<OpenAiSpeechClient>) -> Self {
                Self {
                    id: EntityId::new(id),
                    client,
                }
            }
        }

        impl Entity for $name {
            fn kind(&self) -> EntityKind {
                EntityKind::$kind
            }
            fn entity_id(&self) -> &EntityId {
                &self.id
            }
        }
    };
}

provider!(
    OpenAiVoice,
    Voice,
    "Speech to speech with tool calling over the Realtime WebSocket (R28.3)."
);
provider!(
    OpenAiTranscriber,
    SpeechToText,
    "Streaming transcription over the Realtime transcription session (R28.3)."
);
provider!(
    OpenAiSpeaker,
    TextToSpeech,
    "Synthesis over `/v1/audio/speech`, streamed (R28.4)."
);

harness_async_trait! {
    impl Voice for OpenAiVoice {
        async fn session(&self, config: VoiceSessionConfig) -> Result<Box<dyn VoiceSession>, ModelError> {
            let c = &self.client;
            let url = c.realtime_url(&[("model", &c.realtime_model)])?;
            let socket = c.open(url, realtime_session_config(&c.realtime_model, &c.transcribe_model, &config)).await?;
            Ok(Box::new(OpenAiVoiceSession { socket, seq: 0, ended: false }))
        }
    }
}

/// The transcription session's state while it streams.
struct Transcribing {
    socket: Box<dyn WebSocket>,
    audio: Option<AudioStream>,
    committed: bool,
    done: bool,
}

harness_async_trait! {
    impl SpeechToText for OpenAiTranscriber {
        async fn transcribe(&self, format: AudioFormat, audio: AudioStream) -> Result<HarnessStream<'static, Result<TranscriptEvent, ModelError>>, ModelError> {
            require_pcm24k(format)?;
            let c = &self.client;
            let url = c.realtime_url(&[("intent", "transcription")])?;
            let socket = c.open(url, transcription_session_config(&c.transcribe_model)).await?;
            let state = Transcribing { socket, audio: Some(audio), committed: false, done: false };
            Ok(boxed_stream(futures::stream::unfold(state, |mut s| async move {
                loop {
                    if s.done {
                        return None;
                    }
                    // Pump the audio first; once it ends, commit and read to the end.
                    if let Some(audio) = s.audio.as_mut() {
                        match audio.next().await {
                            Some(Ok(bytes)) => {
                                if let Err(e) = send_json(s.socket.as_mut(), &audio_append(&bytes)).await {
                                    s.done = true;
                                    return Some((Err(e), s));
                                }
                                continue;
                            }
                            Some(Err(e)) => {
                                s.done = true;
                                return Some((Err(host_error(&e)), s));
                            }
                            None => s.audio = None,
                        }
                    }
                    if !s.committed {
                        s.committed = true;
                        if let Err(e) = send_json(s.socket.as_mut(), &json!({ "type": "input_audio_buffer.commit" })).await {
                            s.done = true;
                            return Some((Err(e), s));
                        }
                    }
                    let frame = match s.socket.next().await {
                        None | Some(Ok(WsMessage::Close)) => return None,
                        Some(Err(e)) => {
                            s.done = true;
                            return Some((Err(host_error(&e)), s));
                        }
                        Some(Ok(WsMessage::Text(t))) => t,
                        Some(Ok(_)) => continue,
                    };
                    let item = match parse_realtime(&frame) {
                        Ok(RealtimeEvent::UserTranscriptDelta(text)) => Ok(TranscriptEvent::Delta { text }),
                        Ok(RealtimeEvent::UserTranscript(text)) => {
                            // One committed buffer, one final transcript: the stream ends.
                            s.done = true;
                            let _ = s.socket.close().await;
                            Ok(TranscriptEvent::Final { text })
                        }
                        Ok(RealtimeEvent::Error(message)) => {
                            s.done = true;
                            Err(fatal(message))
                        }
                        Ok(_) => continue,
                        Err(e) => Err(e),
                    };
                    return Some((item, s));
                }
            })))
        }
    }
}

harness_async_trait! {
    impl TextToSpeech for OpenAiSpeaker {
        async fn synthesise(&self, text: String, voice: VoiceId) -> Result<HarnessStream<'static, Result<AudioChunk, ModelError>>, ModelError> {
            let c = &self.client;
            let mut url = c.base_url.clone();
            let path = format!("{}/audio/speech", url.path().trim_end_matches('/'));
            url.set_path(&path);
            let mut request = HttpRequest::new(http::Method::POST, url);
            request.headers = c.auth()?;
            request.headers.insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("application/json"));
            request.body = Bytes::from(
                json!({ "model": c.speech_model, "input": text, "voice": voice.0, "response_format": "pcm" }).to_string(),
            );
            let response = c.http.send_streaming(request).await.map_err(|e| host_error(&e))?;
            if !response.status.is_success() {
                let status = response.status.as_u16();
                let body: Vec<Bytes> = response.body.filter_map(|r| async move { r.ok() }).collect().await;
                let message = String::from_utf8_lossy(&body.concat()).replace(c.api_key.expose().as_str(), "[redacted]");
                return Err(ModelError::from_status(status, message, None));
            }
            let mut seq = 0u64;
            Ok(boxed_stream(response.body.map(move |r| {
                seq += 1;
                r.map(|data| AudioChunk { format: PCM24K, data, seq }).map_err(|e| host_error(&e))
            })))
        }
    }
}

/// The three providers `[voice]` describes, sharing one client.
///
/// # Errors
///
/// See [`OpenAiSpeechClient::from_config`].
pub async fn providers(
    config: &VoiceConfig,
    host: &Host,
) -> Result<(OpenAiVoice, OpenAiTranscriber, OpenAiSpeaker), ModelError> {
    let client = Arc::new(OpenAiSpeechClient::from_config(config, host).await?);
    Ok((
        OpenAiVoice::new("default", Arc::clone(&client)),
        OpenAiTranscriber::new("default", Arc::clone(&client)),
        OpenAiSpeaker::new("default", client),
    ))
}

//! The `SpeechToText`, `TextToSpeech` and `Voice` interfaces and their providers
//! (R28.3–5): the `OpenAI` implementations in [`openai`], the harness-side session a
//! surface streams through in [`relay`], and the `voice/v1` audio parts both use.

pub mod openai;
pub mod relay;

use serde::{Deserialize, Serialize};

use super::llm::ModelError;
use crate::harness_async_trait;
use crate::host::{AudioFormat, AudioStream};
use crate::registry::Entity;
use crate::runtime::HarnessStream;

/// One piece of a transcript as it arrives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "event")]
pub enum TranscriptEvent {
    /// More transcribed text (may revise the previous delta's tail).
    Delta {
        /// The text.
        text: String,
    },
    /// A final segment.
    Final {
        /// The text.
        text: String,
    },
}

/// A chunk of synthesised audio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioChunk {
    /// The encoding.
    pub format: AudioFormat,
    /// The bytes.
    pub data: bytes::Bytes,
    /// Sequence number within the utterance.
    pub seq: u64,
}

/// A voice to synthesise with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceId(pub String);

harness_async_trait! {
    /// Speech to text (R28.3).
    pub trait SpeechToText: Entity {
        /// Transcribes `audio` as it arrives.
        async fn transcribe(&self, format: AudioFormat, audio: AudioStream) -> Result<HarnessStream<'static, Result<TranscriptEvent, ModelError>>, ModelError>;
    }
}

harness_async_trait! {
    /// Text to speech (R28.4).
    pub trait TextToSpeech: Entity {
        /// Synthesises `text` with `voice`, streaming audio as it is produced.
        async fn synthesise(&self, text: String, voice: VoiceId) -> Result<HarnessStream<'static, Result<AudioChunk, ModelError>>, ModelError>;
    }
}

/// What a voice session is configured with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VoiceSessionConfig {
    /// The instructions, rendered.
    pub instructions: String,
    /// The voice.
    pub voice: VoiceId,
    /// The tools the session may call.
    pub tools: Vec<crate::harness::tools::ToolDescriptor>,
}

/// What a voice session emits.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VoiceEvent {
    /// The model's speech.
    Audio(AudioChunk),
    /// The transcript of the model's speech.
    Transcript(String),
    /// The transcript of the user's speech.
    UserTranscript(String),
    /// A tool call to run and answer with [`VoiceSession::tool_result`].
    ToolCall(crate::harness::tools::ToolCall),
    /// The session ended.
    Ended,
}

harness_async_trait! {
    /// An open speech-to-speech session (R28.3, Q15).
    pub trait VoiceSession: Send {
        /// Sends user audio.
        async fn send_audio(&mut self, chunk: AudioChunk) -> Result<(), ModelError>;
        /// Answers a tool call.
        async fn tool_result(&mut self, call_id: String, result: String) -> Result<(), ModelError>;
        /// The next event; `None` once ended.
        async fn next(&mut self) -> Option<Result<VoiceEvent, ModelError>>;
        /// Ends the session.
        async fn close(&mut self) -> Result<(), ModelError>;
    }
}

harness_async_trait! {
    /// Speech to speech with tool calling (R28.3).
    pub trait Voice: Entity {
        /// Opens a session.
        async fn session(&self, config: VoiceSessionConfig) -> Result<Box<dyn VoiceSession>, ModelError>;
    }
}

/// The `voice/v1` extension payload on an audio part (design.md § A2A extensions).
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct VoiceMeta {
    /// The chunk's position in its attempt.
    pub seq: u64,
    /// The attempt that produced it (retracted attempts are dropped by renderers).
    pub attempt: crate::harness::core_loop::task::Attempt,
    /// `pcm16@24k` or `opus`.
    pub format: String,
    /// Whether this is the last chunk of the utterance.
    #[serde(rename = "final")]
    pub is_final: bool,
    /// The voice session.
    pub session: String,
}

/// The `voice/v1` extension URI: the metadata key on an audio part.
pub const VOICE_EXT: &str = "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/voice/v1";
/// The media type of a `voice/v1` PCM audio part.
pub const PCM_MEDIA_TYPE: &str = "audio/pcm";

/// `chunk` as an A2A audio part carrying `meta` under `voice/v1`.
#[must_use]
pub fn audio_part(chunk: &AudioChunk, meta: &VoiceMeta) -> a2a::Part {
    let mut part = a2a::Part::data(serde_json::Value::Null).with_media_type(PCM_MEDIA_TYPE);
    part.content = a2a::PartContent::Raw(chunk.data.to_vec());
    part.metadata = Some(std::collections::HashMap::from([(
        VOICE_EXT.to_owned(),
        serde_json::to_value(meta).unwrap_or(serde_json::Value::Null),
    )]));
    part
}

/// The `voice/v1` metadata and the bytes of an audio part, if it is one.
#[must_use]
pub fn audio_of(part: &a2a::Part) -> Option<(VoiceMeta, bytes::Bytes)> {
    let meta = part.metadata.as_ref()?.get(VOICE_EXT)?;
    let meta: VoiceMeta = serde_json::from_value(meta.clone()).ok()?;
    match &part.content {
        a2a::PartContent::Raw(bytes) => Some((meta, bytes::Bytes::from(bytes.clone()))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! The `OpenAI` speech clients against a scripted `WebSocket` and HTTP client: event
    //! framing, chunk sequencing, tool round trips, the transcription stream, synthesis.

    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use base64::Engine as _;
    use bytes::Bytes;
    use futures::StreamExt as _;
    use serde_json::{Value, json};

    use super::openai::{
        OpenAiSpeaker, OpenAiSpeechClient, OpenAiTranscriber, OpenAiVoice, PCM24K, RealtimeEvent,
        parse_realtime,
    };
    use super::*;
    use crate::harness::tools::ToolDescriptor;
    use crate::host::{
        HostError, HttpClient, HttpRequest, HttpResponse, HttpStreamingResponse, Secret, WebSocket,
        WebSocketClient, WsMessage,
    };
    use crate::runtime::boxed_stream;

    /// What the scripted socket saw: the handshake and every client frame.
    #[derive(Default)]
    struct Seen {
        url: Option<url::Url>,
        headers: http::HeaderMap,
        sent: Vec<Value>,
    }

    struct ScriptedWs {
        seen: Arc<Mutex<Seen>>,
        frames: Arc<Mutex<VecDeque<String>>>,
    }

    struct ScriptedSocket {
        seen: Arc<Mutex<Seen>>,
        frames: Arc<Mutex<VecDeque<String>>>,
    }

    crate::harness_async_trait! {
        impl WebSocketClient for ScriptedWs {
            async fn connect(&self, url: url::Url, headers: http::HeaderMap) -> Result<Box<dyn WebSocket>, HostError> {
                let mut seen = self.seen.lock().unwrap();
                seen.url = Some(url);
                seen.headers = headers;
                Ok(Box::new(ScriptedSocket { seen: Arc::clone(&self.seen), frames: Arc::clone(&self.frames) }))
            }
        }
    }

    crate::harness_async_trait! {
        impl WebSocket for ScriptedSocket {
            async fn send(&mut self, message: WsMessage) -> Result<(), HostError> {
                if let WsMessage::Text(t) = message {
                    self.seen.lock().unwrap().sent.push(serde_json::from_str(&t).unwrap());
                }
                Ok(())
            }
            async fn next(&mut self) -> Option<Result<WsMessage, HostError>> {
                self.frames.lock().unwrap().pop_front().map(|f| Ok(WsMessage::Text(f)))
            }
            async fn close(&mut self) -> Result<(), HostError> {
                Ok(())
            }
        }
    }

    struct ScriptedHttp {
        status: u16,
        chunks: Vec<&'static [u8]>,
        seen: Arc<Mutex<Option<HttpRequest>>>,
    }

    crate::harness_async_trait! {
        impl HttpClient for ScriptedHttp {
            async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, HostError> {
                Err(HostError::Io("not scripted".into()))
            }
            async fn send_streaming(&self, request: HttpRequest) -> Result<HttpStreamingResponse, HostError> {
                *self.seen.lock().unwrap() = Some(request);
                let chunks: Vec<Result<Bytes, HostError>> = self.chunks.iter().map(|c| Ok(Bytes::from_static(c))).collect();
                Ok(HttpStreamingResponse {
                    status: http::StatusCode::from_u16(self.status).unwrap(),
                    headers: http::HeaderMap::new(),
                    body: boxed_stream(futures::stream::iter(chunks)),
                })
            }
        }
    }

    const KEY: &str = "sk-voice-canary-0123456789";

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn client(frames: Vec<Value>) -> (Arc<OpenAiSpeechClient>, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let ws = ScriptedWs {
            seen: Arc::clone(&seen),
            frames: Arc::new(Mutex::new(
                frames.into_iter().map(|f| f.to_string()).collect(),
            )),
        };
        let http = ScriptedHttp {
            status: 200,
            chunks: vec![],
            seen: Arc::new(Mutex::new(None)),
        };
        let c = OpenAiSpeechClient::new(
            "https://api.openai.com/v1".parse().unwrap(),
            Secret::new(KEY.to_owned()),
            Arc::new(http),
            Arc::new(ws),
        );
        (Arc::new(c), seen)
    }

    fn session_config() -> VoiceSessionConfig {
        VoiceSessionConfig {
            instructions: "You draft release notes.".into(),
            voice: VoiceId("marin".into()),
            tools: vec![ToolDescriptor {
                name: "changelog.list_merged".into(),
                description: "Lists merged pull requests.".into(),
                parameters: json!({ "type": "object" }),
                idempotency: crate::harness::tools::Idempotency::Idempotent,
            }],
        }
    }

    #[tokio::test]
    async fn a_voice_session_frames_audio_and_sequences_the_models_chunks() {
        let (c, seen) = client(vec![
            json!({ "type": "session.updated" }),
            json!({ "type": "conversation.item.input_audio_transcription.completed", "transcript": "add the deprecations" }),
            json!({ "type": "response.output_audio.delta", "delta": b64(b"\x01\x02") }),
            json!({ "type": "response.output_audio_transcript.delta", "delta": "Adding" }),
            json!({ "type": "response.output_audio.delta", "delta": b64(b"\x03\x04") }),
            json!({ "type": "response.function_call_arguments.done", "call_id": "call_1", "name": "changelog.list_merged", "arguments": "{\"since\":\"v0.3.0\"}" }),
            json!({ "type": "response.done" }),
        ]);
        let voice = OpenAiVoice::new("default", Arc::clone(&c));
        let mut session = voice.session(session_config()).await.unwrap();
        session
            .send_audio(AudioChunk {
                format: PCM24K,
                data: Bytes::from_static(b"\x0a\x0b"),
                seq: 1,
            })
            .await
            .unwrap();

        let mut events = Vec::new();
        while let Some(e) = session.next().await {
            let e = e.unwrap();
            if let VoiceEvent::ToolCall(call) = &e {
                session
                    .tool_result(call.id.clone(), "25 pull requests".into())
                    .await
                    .unwrap();
            }
            let ended = e == VoiceEvent::Ended;
            events.push(e);
            if ended {
                break;
            }
        }
        assert_eq!(
            events[0],
            VoiceEvent::UserTranscript("add the deprecations".into())
        );
        assert!(
            matches!(&events[1], VoiceEvent::Audio(a) if a.seq == 1 && a.data == Bytes::from_static(b"\x01\x02"))
        );
        assert_eq!(events[2], VoiceEvent::Transcript("Adding".into()));
        assert!(matches!(&events[3], VoiceEvent::Audio(a) if a.seq == 2));
        assert!(
            matches!(&events[4], VoiceEvent::ToolCall(c) if c.id == "call_1" && c.arguments["since"] == "v0.3.0")
        );
        assert_eq!(events.last(), Some(&VoiceEvent::Ended));

        let seen = seen.lock().unwrap();
        let url = seen.url.as_ref().unwrap();
        assert_eq!(url.scheme(), "wss");
        assert_eq!(url.path(), "/v1/realtime");
        assert_eq!(url.query(), Some("model=gpt-realtime-2.1"));
        assert_eq!(
            seen.headers[http::header::AUTHORIZATION],
            format!("Bearer {KEY}")
        );
        assert!(seen.headers[http::header::AUTHORIZATION].is_sensitive());
        assert_eq!(seen.sent[0]["type"], "session.update");
        assert_eq!(seen.sent[0]["session"]["type"], "realtime");
        assert_eq!(
            seen.sent[1],
            json!({ "type": "input_audio_buffer.append", "audio": b64(b"\x0a\x0b") })
        );
        assert_eq!(
            seen.sent[2]["item"],
            json!({ "type": "function_call_output", "call_id": "call_1", "output": "25 pull requests" })
        );
        assert_eq!(seen.sent[3], json!({ "type": "response.create" }));
    }

    #[tokio::test]
    async fn a_session_error_is_a_model_error_and_other_formats_are_refused() {
        let (c, _) = client(vec![
            json!({ "type": "error", "error": { "type": "invalid_request_error", "message": "bad audio" } }),
        ]);
        let voice = OpenAiVoice::new("default", c);
        let mut session = voice.session(session_config()).await.unwrap();
        let err = session.next().await.unwrap().unwrap_err();
        assert!(err.to_string().contains("bad audio"), "{err}");
        let opus = session
            .send_audio(AudioChunk {
                format: crate::host::AudioFormat::Opus,
                data: Bytes::new(),
                seq: 1,
            })
            .await;
        assert!(matches!(
            opus,
            Err(crate::harness::models::llm::ModelError::NonRetryable { .. })
        ));
        assert_eq!(
            parse_realtime("{\"type\":\"rate_limits.updated\"}").unwrap(),
            RealtimeEvent::Other
        );
        assert!(parse_realtime("not json").is_err());
    }

    #[tokio::test]
    async fn transcription_streams_deltas_commits_after_the_audio_and_ends_on_the_final() {
        let (c, seen) = client(vec![
            json!({ "type": "conversation.item.input_audio_transcription.delta", "delta": "add the " }),
            json!({ "type": "conversation.item.input_audio_transcription.delta", "delta": "deprecations" }),
            json!({ "type": "conversation.item.input_audio_transcription.completed", "transcript": "add the deprecations" }),
            json!({ "type": "conversation.item.input_audio_transcription.delta", "delta": "never read" }),
        ]);
        let stt = OpenAiTranscriber::new("default", Arc::clone(&c));
        let audio = boxed_stream(futures::stream::iter(vec![
            Ok(Bytes::from_static(b"\x01")),
            Ok(Bytes::from_static(b"\x02")),
        ]));
        let events: Vec<TranscriptEvent> = stt
            .transcribe(PCM24K, audio)
            .await
            .unwrap()
            .map(Result::unwrap)
            .collect()
            .await;
        assert_eq!(
            events,
            [
                TranscriptEvent::Delta {
                    text: "add the ".into()
                },
                TranscriptEvent::Delta {
                    text: "deprecations".into()
                },
                TranscriptEvent::Final {
                    text: "add the deprecations".into()
                },
            ]
        );
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.url.as_ref().unwrap().query(),
            Some("intent=transcription")
        );
        let kinds: Vec<&str> = seen
            .sent
            .iter()
            .map(|f| f["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            [
                "session.update",
                "input_audio_buffer.append",
                "input_audio_buffer.append",
                "input_audio_buffer.commit"
            ]
        );
        assert_eq!(
            seen.sent[0]["session"]["audio"]["input"]["transcription"]["model"],
            "gpt-live-transcribe"
        );
    }

    #[tokio::test]
    async fn synthesis_streams_pcm_chunks_and_never_echoes_the_key_in_an_error() {
        let seen_req = Arc::new(Mutex::new(None));
        let http = ScriptedHttp {
            status: 200,
            chunks: vec![b"\x01\x02", b"\x03"],
            seen: Arc::clone(&seen_req),
        };
        let ws = ScriptedWs {
            seen: Arc::new(Mutex::new(Seen::default())),
            frames: Arc::new(Mutex::new(VecDeque::new())),
        };
        let c = Arc::new(OpenAiSpeechClient::new(
            "https://api.openai.com/v1".parse().unwrap(),
            Secret::new(KEY.to_owned()),
            Arc::new(http),
            Arc::new(ws),
        ));
        let tts = OpenAiSpeaker::new("default", c);
        let chunks: Vec<AudioChunk> = tts
            .synthesise("hello".into(), VoiceId("marin".into()))
            .await
            .unwrap()
            .map(Result::unwrap)
            .collect()
            .await;
        assert_eq!(chunks.iter().map(|c| c.seq).collect::<Vec<_>>(), [1, 2]);
        assert!(chunks.iter().all(|c| c.format == PCM24K));
        let request = seen_req.lock().unwrap().take().unwrap();
        assert_eq!(
            request.url.as_str(),
            "https://api.openai.com/v1/audio/speech"
        );
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body,
            json!({ "model": "gpt-4o-mini-tts", "input": "hello", "voice": "marin", "response_format": "pcm" })
        );

        let http = ScriptedHttp {
            status: 401,
            chunks: vec![b"invalid key sk-voice-canary-0123456789"],
            seen: Arc::new(Mutex::new(None)),
        };
        let ws = ScriptedWs {
            seen: Arc::new(Mutex::new(Seen::default())),
            frames: Arc::new(Mutex::new(VecDeque::new())),
        };
        let c = Arc::new(OpenAiSpeechClient::new(
            "https://api.openai.com/v1".parse().unwrap(),
            Secret::new(KEY.to_owned()),
            Arc::new(http),
            Arc::new(ws),
        ));
        let Err(err) = OpenAiSpeaker::new("default", c)
            .synthesise("hi".into(), VoiceId("marin".into()))
            .await
        else {
            panic!("a 401 is an error")
        };
        assert!(!err.to_string().contains(KEY), "{err}");
        assert!(err.to_string().contains("[redacted]"), "{err}");
    }

    #[test]
    fn an_audio_part_carries_its_voice_meta_and_reads_back() {
        let chunk = AudioChunk {
            format: PCM24K,
            data: Bytes::from_static(b"\x01\x02"),
            seq: 7,
        };
        let meta = VoiceMeta {
            seq: 7,
            attempt: crate::harness::core_loop::task::Attempt { iteration: 2, n: 1 },
            format: "pcm16@24k".into(),
            is_final: false,
            session: "s1".into(),
        };
        let part = audio_part(&chunk, &meta);
        assert_eq!(part.media_type.as_deref(), Some(PCM_MEDIA_TYPE));
        assert_eq!(
            audio_of(&part),
            Some((meta, Bytes::from_static(b"\x01\x02")))
        );
        assert_eq!(audio_of(&a2a::Part::text("x")), None);
    }
}

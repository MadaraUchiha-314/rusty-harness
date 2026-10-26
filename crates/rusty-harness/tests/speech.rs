//! Feature: Voice through the harness
//! Requirement: docs/specs/issue-3/requirements.md#R28 (R28.3–6), boundary 6, abuse 11
//!
//! The task 39 checks that need the public API: the Realtime session configuration
//! snapshot (T6) and the voice leg of the credential canary (T8): a surface streaming
//! audio through the relay never sees the provider key, the provider URL or a frame
//! that carries either.
#![cfg(feature = "rt-tokio")]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use bytes::Bytes;
use futures::StreamExt as _;
use rusty_harness::harness::core_loop::task::Attempt;
use rusty_harness::harness::models::speech::openai::{
    OpenAiSpeechClient, OpenAiVoice, PCM24K, realtime_session_config,
};
use rusty_harness::harness::models::speech::relay::{RelayItem, ToolRunner, relay};
use rusty_harness::harness::models::speech::{
    AudioChunk, VOICE_EXT, VoiceId, VoiceSessionConfig, audio_of,
};
use rusty_harness::harness::tools::{Idempotency, ToolDescriptor};
use rusty_harness::harness_async_trait;
use rusty_harness::host::{
    HostError, HttpClient, HttpRequest, HttpResponse, HttpStreamingResponse, Secret, WebSocket,
    WebSocketClient, WsMessage,
};
use serde_json::{Value, json};

const KEY: &str = "sk-voice-canary-relay-9876543210";

fn config() -> VoiceSessionConfig {
    VoiceSessionConfig {
        instructions: "You are demo-agent. Draft release notes with the person.".into(),
        voice: VoiceId("marin".into()),
        tools: vec![
            ToolDescriptor {
                name: "changelog.list_merged".into(),
                description: "Lists the pull requests merged since a tag.".into(),
                parameters: json!({ "type": "object", "properties": { "since": { "type": "string" } }, "required": ["since"] }),
                idempotency: Idempotency::Idempotent,
            },
            ToolDescriptor {
                name: "participant.ask".into(),
                description: "Asks a participant.".into(),
                parameters: json!({ "type": "object" }),
                idempotency: Idempotency::SideEffecting { accepts_key: true },
            },
        ],
    }
}

/// T6: the `session.update` body the harness opens a voice session with.
#[test]
fn snapshot_realtime_session_config() {
    let mut json = serde_json::to_string_pretty(&realtime_session_config(
        "gpt-realtime-2.1",
        "gpt-live-transcribe",
        &config(),
    ))
    .unwrap();
    json.push('\n');
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots/realtime_session_config.json");
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &json).unwrap();
    }
    let published: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default())
            .unwrap_or(Value::Null);
    assert_eq!(
        published,
        serde_json::from_str::<Value>(&json).unwrap(),
        "realtime_session_config.json is stale: rerun with UPDATE_SNAPSHOTS=1"
    );
}

#[derive(Default)]
struct Wire {
    connected: Vec<(String, String)>,
    sent: Vec<String>,
}

struct Provider {
    wire: Arc<Mutex<Wire>>,
    script: Arc<Mutex<VecDeque<String>>>,
}

struct Socket {
    wire: Arc<Mutex<Wire>>,
    script: Arc<Mutex<VecDeque<String>>>,
}

harness_async_trait! {
    impl WebSocketClient for Provider {
        async fn connect(&self, url: url::Url, headers: http::HeaderMap) -> Result<Box<dyn WebSocket>, HostError> {
            let auth = headers.get(http::header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
            self.wire.lock().unwrap().connected.push((url.to_string(), auth));
            Ok(Box::new(Socket { wire: Arc::clone(&self.wire), script: Arc::clone(&self.script) }))
        }
    }
}

harness_async_trait! {
    impl WebSocket for Socket {
        async fn send(&mut self, message: WsMessage) -> Result<(), HostError> {
            if let WsMessage::Text(t) = message {
                self.wire.lock().unwrap().sent.push(t);
            }
            Ok(())
        }
        async fn next(&mut self) -> Option<Result<WsMessage, HostError>> {
            // Let the relay pump the person's audio before the model answers.
            tokio::task::yield_now().await;
            self.script.lock().unwrap().pop_front().map(|f| Ok(WsMessage::Text(f)))
        }
        async fn close(&mut self) -> Result<(), HostError> {
            Ok(())
        }
    }
}

struct NoHttp;

harness_async_trait! {
    impl HttpClient for NoHttp {
        async fn send(&self, _r: HttpRequest) -> Result<HttpResponse, HostError> {
            Err(HostError::Denied("no http in this test".into()))
        }
        async fn send_streaming(&self, _r: HttpRequest) -> Result<HttpStreamingResponse, HostError> {
            Err(HostError::Denied("no http in this test".into()))
        }
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Scenario: The voice session's key never leaves the harness
///     Given a voice provider whose key is a canary value, held by the harness
///     When a surface streams audio through the relay and the model speaks, transcribes
///     and calls a tool
///     Then the surface receives voice/v1 audio parts, transcripts and the tool outcome
///     And neither the key nor the provider URL appears in anything the surface receives
///     And the key appears exactly once on the wire: the handshake's Authorization header
#[tokio::test]
#[allow(clippy::too_many_lines)] // one scenario: the script, the relay, both sides of the wire
async fn abuse_voice_key_never_leaves_the_harness() {
    let wire = Arc::new(Mutex::new(Wire::default()));
    let script: VecDeque<String> = [
        json!({ "type": "session.updated", "session": { "id": "sess_1" } }),
        json!({ "type": "conversation.item.input_audio_transcription.completed", "transcript": "list what merged since v0.3.0" }),
        json!({ "type": "response.function_call_arguments.done", "call_id": "call_9", "name": "changelog.list_merged", "arguments": "{\"since\":\"v0.3.0\"}" }),
        json!({ "type": "response.output_audio_transcript.delta", "delta": "Twenty-five pull requests." }),
        json!({ "type": "response.output_audio.delta", "delta": b64(b"\x10\x20\x30\x40") }),
        json!({ "type": "response.output_audio.delta", "delta": b64(b"\x50\x60") }),
        json!({ "type": "response.done" }),
    ]
    .into_iter()
    .map(|v| v.to_string())
    .collect();
    let provider = Provider {
        wire: Arc::clone(&wire),
        script: Arc::new(Mutex::new(script)),
    };
    let client = Arc::new(OpenAiSpeechClient::new(
        "https://api.openai.com/v1".parse().unwrap(),
        Secret::new(KEY.to_owned()),
        Arc::new(NoHttp),
        Arc::new(provider),
    ));
    let voice = OpenAiVoice::new("default", client);
    let mic = futures::stream::iter(vec![
        AudioChunk {
            format: PCM24K,
            data: Bytes::from_static(b"\x01\x02"),
            seq: 1,
        },
        AudioChunk {
            format: PCM24K,
            data: Bytes::from_static(b"\x03\x04"),
            seq: 2,
        },
    ])
    .boxed();
    let run_tool: ToolRunner = Arc::new(|call| {
        Box::pin(async move {
            format!(
                "{} ran with {}: 25 pull requests",
                call.name, call.arguments
            )
        })
    });
    let attempt = Attempt { iteration: 3, n: 1 };
    let items: Vec<RelayItem> = relay(&voice, config(), "voice-1".into(), attempt, mic, run_tool)
        .await
        .unwrap()
        .map(Result::unwrap)
        .collect()
        .await;

    // What the surface receives.
    assert!(items.contains(&RelayItem::UserTranscript(
        "list what merged since v0.3.0".into()
    )));
    assert!(items.contains(&RelayItem::AgentTranscript(
        "Twenty-five pull requests.".into()
    )));
    assert!(items.iter().any(|i| matches!(i, RelayItem::ToolCalled { call, result } if call.id == "call_9" && result.contains("25 pull requests"))));
    let audio: Vec<_> = items
        .iter()
        .filter_map(|i| match i {
            RelayItem::Audio(part) => audio_of(part),
            _ => None,
        })
        .collect();
    assert_eq!(audio.len(), 2);
    assert_eq!(audio[0].0.seq, 1);
    assert_eq!(audio[1].0.seq, 2);
    assert_eq!(audio[0].0.attempt, attempt);
    assert_eq!(audio[0].0.session, "voice-1");
    assert_eq!(audio[0].1, Bytes::from_static(b"\x10\x20\x30\x40"));
    assert_eq!(items.last(), Some(&RelayItem::Ended));
    let surface = format!("{items:?}");
    assert!(!surface.contains(KEY), "the key reached the surface");
    assert!(
        !surface.contains("api.openai.com"),
        "the provider URL reached the surface"
    );
    let parts_json: Vec<Value> = items
        .iter()
        .filter_map(|i| match i {
            RelayItem::Audio(p) => serde_json::to_value(p).ok(),
            _ => None,
        })
        .collect();
    assert!(
        parts_json
            .iter()
            .all(|p| p["metadata"][VOICE_EXT]["format"] == "pcm16@24k")
    );
    assert!(!serde_json::to_string(&parts_json).unwrap().contains(KEY));

    // What went to the provider: the key once, in the handshake, never in a frame.
    let wire = wire.lock().unwrap();
    assert_eq!(wire.connected.len(), 1);
    assert_eq!(wire.connected[0].1, format!("Bearer {KEY}"));
    assert!(
        wire.sent.iter().all(|f| !f.contains(KEY)),
        "a frame carried the key"
    );
    let appended: Vec<&String> = wire
        .sent
        .iter()
        .filter(|f| f.contains("input_audio_buffer.append"))
        .collect();
    assert_eq!(
        appended.len(),
        2,
        "both microphone chunks reached the provider: {:?}",
        wire.sent
    );
    assert!(
        wire.sent
            .iter()
            .any(|f| f.contains("function_call_output") && f.contains("call_9"))
    );
}

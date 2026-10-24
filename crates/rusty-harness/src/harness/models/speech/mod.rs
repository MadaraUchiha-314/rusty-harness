//! The `SpeechToText`, `TextToSpeech` and `Voice` interfaces and their providers
//! (R28.3–5). The interfaces land with the models (task 11); the `OpenAI`
//! implementations land with milestone 6 (task 39).

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

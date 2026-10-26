//! The harness-side voice session (R28.3, R28.5–6, D7): a surface streams microphone
//! audio to the harness as `voice/v1` parts, the harness holds the provider session and
//! its key, runs the session's tool calls through its own executor, and streams the
//! model's audio and the transcripts back. No credential and no provider URL ever reaches
//! the surface (boundary 6, abuse 11).

use std::pin::pin;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::future::Either;

use super::{
    AudioChunk, Voice, VoiceEvent, VoiceMeta, VoiceSession, VoiceSessionConfig, audio_part,
};
use crate::harness::core_loop::task::Attempt;
use crate::harness::models::llm::ModelError;
use crate::harness::tools::ToolCall;
use crate::runtime::{HarnessFuture, HarnessStream, boxed_stream};

/// Runs a tool call for the session and answers with its rendered result.
pub type ToolRunner = Arc<dyn Fn(ToolCall) -> HarnessFuture<'static, String> + Send + Sync>;

/// What the relay hands the surface, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum RelayItem {
    /// The model's speech, as a `voice/v1` audio part.
    Audio(a2a::Part),
    /// What the person said, transcribed.
    UserTranscript(String),
    /// What the model said, transcribed (deltas).
    AgentTranscript(String),
    /// The model called a tool; the harness ran it and answered.
    ToolCalled {
        /// The call.
        call: ToolCall,
        /// The result the session received.
        result: String,
    },
    /// The session ended.
    Ended,
}

struct State {
    session: Box<dyn VoiceSession>,
    inbound: Option<HarnessStream<'static, AudioChunk>>,
    run_tool: ToolRunner,
    session_id: String,
    attempt: Attempt,
    ended: bool,
}

/// Opens a session on `voice` and relays `inbound` audio into it, yielding what the
/// surface shows and plays. When `inbound` ends the session stays open until the
/// provider ends it or the caller drops the stream.
///
/// # Errors
///
/// The session could not be opened.
pub async fn relay(
    voice: &dyn Voice,
    config: VoiceSessionConfig,
    session_id: String,
    attempt: Attempt,
    inbound: HarnessStream<'static, AudioChunk>,
    run_tool: ToolRunner,
) -> Result<HarnessStream<'static, Result<RelayItem, ModelError>>, ModelError> {
    let session = voice.session(config).await?;
    let state = State {
        session,
        inbound: Some(inbound),
        run_tool,
        session_id,
        attempt,
        ended: false,
    };
    Ok(boxed_stream(futures::stream::unfold(state, step)))
}

enum Next {
    Audio(Option<AudioChunk>),
    Event(Option<Result<VoiceEvent, ModelError>>),
}

async fn step(mut s: State) -> Option<(Result<RelayItem, ModelError>, State)> {
    loop {
        if s.ended {
            return None;
        }
        // Whichever is ready first: more of the person's audio, or the model. The two
        // futures live only inside this block, so the session is free afterwards.
        let next = if let Some(inbound) = s.inbound.as_mut() {
            let next_audio = pin!(inbound.next());
            let next_event = pin!(s.session.next());
            match futures::future::select(next_audio, next_event).await {
                Either::Left((chunk, _)) => Next::Audio(chunk),
                Either::Right((event, _)) => Next::Event(event),
            }
        } else {
            Next::Event(s.session.next().await)
        };
        let event = match next {
            Next::Audio(Some(chunk)) => {
                if let Err(e) = s.session.send_audio(chunk).await {
                    return Some((Err(e), s));
                }
                continue;
            }
            Next::Audio(None) => {
                s.inbound = None;
                continue;
            }
            Next::Event(event) => event,
        };
        let item = match event {
            None | Some(Ok(VoiceEvent::Ended)) => {
                s.ended = true;
                Ok(RelayItem::Ended)
            }
            Some(Err(e)) => Err(e),
            Some(Ok(VoiceEvent::Audio(chunk))) => {
                let meta = VoiceMeta {
                    seq: chunk.seq,
                    attempt: s.attempt,
                    format: "pcm16@24k".into(),
                    is_final: false,
                    session: s.session_id.clone(),
                };
                Ok(RelayItem::Audio(audio_part(&chunk, &meta)))
            }
            Some(Ok(VoiceEvent::Transcript(t))) => Ok(RelayItem::AgentTranscript(t)),
            Some(Ok(VoiceEvent::UserTranscript(t))) => Ok(RelayItem::UserTranscript(t)),
            Some(Ok(VoiceEvent::ToolCall(call))) => {
                let result = (s.run_tool)(call.clone()).await;
                if let Err(e) = s.session.tool_result(call.id.clone(), result.clone()).await {
                    return Some((Err(e), s));
                }
                Ok(RelayItem::ToolCalled { call, result })
            }
        };
        return Some((item, s));
    }
}

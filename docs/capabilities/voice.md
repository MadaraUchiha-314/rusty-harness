# Capability: multi-modal input and output, and voice

> Images and audio in, images and speech out, on every surface; the harness holds every
> provider session and its key, and a model gets only what it can take.

## What it is

A participant's message may carry image and audio parts (raw bytes with a media type).
Images reach the selected model as image content parts. Audio reaches it as audio parts
when it can hear; otherwise a `voice/v1` audio part is transcribed by the registered
`SpeechToText` first. Media the model cannot take is refused before any task exists, as
`ContentTypeNotSupported` naming the modality and the model (R28.1).

Out, a tool can return images or files as artifacts (`ToolOutput::Artifacts`), and the
reply to a spoken turn is also synthesised into an `audio/pcm` artifact by the registered
`TextToSpeech` (R28.2, R28.4). Both renderers show image artifacts (the web as `<img>`,
the TUI through the terminal's graphics protocol or as alt text) and play spoken
replies.

`[voice] provider = "openai-realtime"` registers three providers on one key: `Voice`
(`gpt-realtime-2.1` over the Realtime WebSocket, with tool calls run by the harness),
`SpeechToText` (`gpt-live-transcribe`), `TextToSpeech` (`/v1/audio/speech`). The relay
(`models::speech::relay`) runs a speech-to-speech session for a surface. Audio is 16-bit
PCM at 24 kHz mono everywhere; `voice/v1` metadata (`seq`, `attempt`, `format`, `final`,
`session`) rides on each audio part. See
[decision-005](../decisions/decision-005.md) for why voice goes through the harness.

## Current behaviour

- WHEN a message carries an image or audio part the selected model takes THEN the part
  SHALL reach the model after the message's text (R28.1).
- WHEN it carries media the model cannot take THEN the message SHALL be refused as
  `ContentTypeNotSupported` naming the modality and the model, unless it is `voice/v1`
  audio and a `SpeechToText` is registered, which transcribes it (R28.1, R28.3).
- WHEN a tool returns artifacts THEN the participants SHALL receive them in the same
  commit as the turn, and the model SHALL see only their names (R28.2).
- WHEN the participant's last turn was spoken and a `TextToSpeech` is registered THEN the
  reply SHALL also arrive as an `audio/pcm` artifact in `[voice] voice` (R28.4).
- WHEN a surface streams audio through the relay THEN neither the key nor the provider URL
  SHALL reach it (boundary 6, abuse 11).
- WHEN a speech provider is added THEN it SHALL implement the same three traits, and no
  surface code SHALL change (R28.5).

## Design

- [`design.md` § Multi-modal input and output][design] (R28), trade-off T6, D7.
- Code: `crates/rusty-harness/src/harness/models/speech/` (`openai`, `relay`), the
  spoken-turn steps in `harness/core_loop/agent.rs` (`hear`, `speak`),
  `host/audio.rs` (microphone and speaker on `cpal`), `host/native.rs` (WebSockets).
- Surfaces: [the terminal renderer](tui.md) (push-to-talk), [the web renderer](web.md)
  (the voice button). Manual procedures: `docs/specs/issue-3/evidence/manual.md`.

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M6, tasks 38–41) | Image and audio parts, tool artifacts, the OpenAI speech providers, the relay, spoken turns, voice on the TUI and the web | PR #12 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

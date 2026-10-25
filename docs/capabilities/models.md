# Capability: models, LLM and System One

> One `Llm` interface over every provider, streamed, retried and modality-checked; a
> `SystemOne` fast judge beside it; `/model` and `/effort` switch the next call.

## What it is

The loop talks to a model through one trait, `Llm::stream(ModelRequest)`, which yields
text deltas, complete tool calls, usage and a stop reason. Two thin clients implement it
over each provider's HTTP API (OpenAI Chat Completions and anything compatible through
`base_url`; Anthropic Messages with cache breakpoints), both built for native and
`wasm32` from the same code (Q16). `Retrying` wraps any model with the configured
backoff on retryable failures; `ModelRouter` resolves the alias the task's selection
names, so a command can switch models between calls without a restart.

```mermaid
flowchart LR
  R["ModelRequest (Prompt + messages + tools)"] --> M["Llm::stream"] --> E["TextDelta · ToolCall · Usage · Stop"] --> T["collect → ModelTurn"]
  S["SystemOne::judge"] --> V["Verdict (fast classification)"]
```

## Current behaviour

- WHEN a model is configured under `[models.<alias>]` THEN the system SHALL build the
  provider's client with its credential resolved from a `SecretRef` and register it
  under the alias; `default` is the alias the loop starts with.
- WHEN a request carries a modality the model lacks THEN the system SHALL fail before
  any event with a non-retryable error naming the modality.
- WHEN a participant's message carries an image or audio part THEN the part SHALL reach
  the selected model as a content part after the message's text; WHEN that model does
  not accept the modality THEN the message SHALL be refused before any task exists, as
  `ContentTypeNotSupported` naming the modality and the model (R28.1). The card lists
  the image and audio types the harness carries; the model decides.
- WHEN `[voice] provider = "openai-realtime"` is configured THEN the harness SHALL
  register a `Voice` (`gpt-realtime-2.1` over the Realtime WebSocket, tool calls run by
  the harness), a `SpeechToText` (`gpt-live-transcribe` over the transcription session)
  and a `TextToSpeech` (`/v1/audio/speech`, PCM streamed), all on one key resolved by the
  host; audio is 16-bit PCM at 24 kHz mono both ways (R28.3–5).
- WHEN a surface speaks through the harness THEN the relay SHALL open the provider
  session, forward the surface's audio, run the session's tool calls, and hand back
  `voice/v1` audio parts (`seq`, `attempt`, `format`, `session`) and transcripts; the key
  and the provider URL SHALL appear only on the harness's own handshake (boundary 6,
  abuse 11, D7).
- WHEN a participant's message carries `voice/v1` audio and the selected model does not
  take audio THEN the registered `SpeechToText` SHALL transcribe it first, the log
  keeping the transcript marked as spoken; WHEN the participant's last turn was spoken
  THEN the reply SHALL also arrive as an `audio/pcm` artifact synthesised in `[voice]
  voice`, a synthesis failure leaving the text reply alone (R28.3–4).
- WHEN a provider answers with a retryable status THEN `Retrying` SHALL wait the
  configured backoff (honouring `Retry-After`) and try again up to `max_attempts`.
- WHEN `/model <alias>` or `/effort <level>` runs THEN the next call SHALL use the new
  selection; nothing restarts.
- WHEN the context builder places a cache breakpoint THEN the Anthropic client SHALL set
  `cache_control` there and the OpenAI client SHALL keep the prefix stable.
- WHEN a System One judgement is needed THEN the system SHALL ask the configured HTTP
  classifier or, absent one, the LLM on the `system-one/classify` fragment.
- WHEN a provider refuses with a body that echoes the credential THEN the error SHALL
  carry `[redacted]` where the credential was (abuse 7).

## Design

- [`design.md` § Models][design] (R9).
- Code: `crates/rusty-harness/src/harness/models/`, `crates/rusty-harness/src/harness/commands/` (`ModelRouter`).

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M6, task 40) | Spoken turns: transcription before the model, synthesis of the reply | PR #12 |
| issue-3 (M6, task 39) | OpenAI `Voice`, `SpeechToText`, `TextToSpeech`; the native WebSocket; the harness-side relay and `voice/v1` parts | PR #12 |
| issue-3 (M6, task 38) | Image and audio parts from messages into the context; the up-front modality refusal | PR #12 |
| issue-3 (M2, tasks 11, 13) | `Llm`, OpenAI and Anthropic clients, `Retrying`, System One, the model router behind `/model` and `/effort` | [spec](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/), PR #8 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

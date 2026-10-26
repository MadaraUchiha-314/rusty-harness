# Decision 005: voice goes through the harness, as A2A audio parts

- **Status:** accepted
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3)
- **Design:** [`docs/specs/issue-3/design.md`][design] § Multi-modal input and output,
  trade-off T6, open question D7; requirements R28.3–6, boundary 6, abuse 11

## Decision

Every voice surface talks to the **harness**, never to the provider. A surface sends what
it heard as `voice/v1` audio parts on ordinary A2A messages (16-bit PCM at 24 kHz mono),
and receives spoken replies as `audio/pcm` artifacts. The harness holds the provider
sessions and the one key they use:

- a spoken message is transcribed by the registered `SpeechToText` when the selected
  model cannot take audio, and the reply to a spoken turn is synthesised by the
  registered `TextToSpeech`;
- a speech-to-speech `Voice` session (`gpt-realtime-2.1`) is driven by the harness-side
  relay, which forwards the surface's audio, runs the session's tool calls through the
  harness, and hands back `voice/v1` audio parts and transcripts.

The browser-direct WebRTC path with per-session ephemeral keys is not built.

## Why

- **No key leaves the server** (boundary 6, abuse 11): the key rides on the harness's own
  WebSocket handshake or HTTP request and nowhere else; the voice leg of the credential
  canary (`abuse_voice_key_never_leaves_the_harness`) checks it.
- **One path for every surface:** the TUI, the web renderer and any other A2A client speak
  the same parts, so voice needed no surface-specific protocol and no second auth story.
- **The loop stays in charge:** transcripts enter the history, tool calls go through the
  harness's executor, hooks and policy, and the task log records the spoken turn like
  any other.

## Consequences

- Round trips through the harness add latency (T6 estimated 200–400 ms) compared with a
  browser talking to the provider directly; acceptable for push-to-talk turns.
- Surfaces resample to 24 kHz PCM themselves (the web renderer with an
  `OfflineAudioContext`, the TUI on `cpal`); the Linux build of the `audio` feature needs
  ALSA headers.
- An ephemeral-key WebRTC extension stays possible later without changing `voice/v1`.

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

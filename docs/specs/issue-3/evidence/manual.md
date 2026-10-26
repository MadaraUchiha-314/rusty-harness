# Manual procedures (T11)

What automation cannot reach: a real microphone and speaker, a real voice provider, and
the demo recording. Each procedure is run by a person at the verification node, who
fills in **Outcome** and **When**; until then each one reads _not yet executed_.

## Environment

- A machine with a microphone and speakers; Linux needs ALSA (`libasound2`).
- `rusty-harness` built with `--features cli` (which includes `audio`).
- `~/.rusty-harness/config.toml` with a default model and, for voice:

  ```toml
  [voice]
  provider = "openai-realtime"
  api_key = "env:OPENAI_API_KEY"
  ```

- `OPENAI_API_KEY` set in the environment (by reference only; never written down here).

## Status at the verification node (2026-10-25)

V4 is executed. V1–V3 are **not executed**: each needs a person speaking into a real
microphone and listening to speakers, which an unattended session cannot do. The
operator was asked on the issue ([question][t11-question]) whether to run them, keep
them recorded as not executed, or replan T11. The operator chose to keep them recorded
as not executed and to run them after the merge ([decision][t11-decision]); the testing
plan's T11 activity was replanned to V4, with V1–V3 deferred until after the merge
([replan][t11-replan]). What does run automatically: the TUI scenario that sends a
capture as a voice part and queues the spoken reply for playback, the Realtime
session-config snapshot, the voice-key abuse test, and the voice states of the TUI and
the web renderer (the utterance sent as 24 kHz PCM).

## V1 — TUI push-to-talk, hold (kitty keyboard protocol)

1. Run `rusty-harness` in a terminal that implements the kitty keyboard protocol
   (kitty, WezTerm, foot, Ghostty).
2. Send a typed first message so a task exists; wait for `input-required`.
3. With the composer empty, hold space. The composer shows
   `● listening 0:0N · release space to send · esc to drop` and the hints
   `space held (kitty keyboard protocol)`.
4. Say "add a line about the deprecations", release space.

**Expected:** a `🎙 spoken message` turn appears, the agent answers in text, and the
answer is spoken through the speakers; the transcript reached the model (the answer is
about deprecations).

- **Outcome:** _not yet executed_
- **When:** —

## V2 — TUI push-to-talk, toggle (no key releases)

1. Run `rusty-harness` in a terminal without the kitty keyboard protocol (GNOME Terminal,
   xterm).
2. Press space once on an empty composer; the line reads `press space again to send`.
3. Speak, press space again.

**Expected:** as V1. Also: `esc` while listening drops the capture and sends nothing;
space with text in the composer types a space.

- **Outcome:** _not yet executed_
- **When:** —

## V3 — Web voice, capture and playback

1. Start `rusty-harness --no-tui`, open the web renderer with `?url=` pointing at it.
2. Send a typed first message; wait for `input-required`.
3. Click the microphone button, allow the browser's microphone prompt. The composer
   reads `Listening… click again to send · Esc to drop`.
4. Speak, click the button again.

**Expected:** a `🎙 spoken message` turn, a text answer, and the answer played through
the speakers once. The browser's network panel shows only requests to the harness, never
to the provider.

- **Outcome:** _not yet executed_
- **When:** —

## V4 — The demo recording (R32.6)

Recorded with task 46: the definition-of-done demo on the TUI and the web against one
harness, captured as a screen recording.

- **Outcome:** executed. The demo harness, the web renderer and the TUI (in a browser
  terminal through `ttyd`, attached with `--url`) side by side, each working a task
  through the A2UI ask to the draft:
  [`e2e/demo-both-renderers.webm`](e2e/demo-both-renderers.webm), last frame
  [`e2e/demo-both-renderers.png`](e2e/demo-both-renderers.png)
  (`web/tests/e2e/demo-recording.spec.ts`, passed in 25.5 s).
- **When:** 2026-10-25 (task 46)

[t11-decision]: https://github.com/MadaraUchiha-314/rusty-harness/issues/3#issuecomment-6409132309
[t11-question]: https://github.com/MadaraUchiha-314/rusty-harness/issues/3#issuecomment-6407596204
[t11-replan]: https://github.com/MadaraUchiha-314/rusty-harness/issues/3#issuecomment-6409811995

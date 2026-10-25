# Capability: the terminal renderer

> One command opens a terminal UI that is an A2A client of the harness it just started,
> or of a remote one; it renders the stream, the A2UI basic catalog natively, images where
> the terminal can draw them, and every word it shows comes from the string catalog.

## What it is

`rusty-harness` starts the server and, unless `--no-tui` is given, opens the TUI against
the server's loopback URL; `--url <a2a-url>` opens the same TUI against a remote harness
and starts no server (R27.3). The TUI (`interaction::renderer::tui`, feature `tui`) is
built on ratatui 0.30 and crossterm 0.29: a header with the agent, task, context, state
and sub-state; the conversation (turns as markdown through `tui-markdown`, A2UI asks as
forms, artifacts, notices); the plan strip read from the `plan/v1` extension; the
composer; the key hints with the current model, effort and durable mode on the right.
`App` is the state and reducer — stream events in, `Outgoing` requests (`SendStreamingMessage`,
`CancelTask`) out — so every state is testable in a `TestBackend` buffer; `run` drives a
real terminal with a crossterm event stream and the official A2A client.

Controls (R27.4, R21.7): `enter` sends; `^c` opens the complete confirmation, whose `y`
sends the task extension's `transition: complete` message, `n` keeps the task open and
`x` cancels instead; `^x` sends `CancelTask`; `^d` leaves. A message after a terminal
task starts a new task in the same context with `referenceTaskIds` naming the old one.
`/model` with the in-process harness opens a picker over the configured aliases (their
tool, vision and voice traits read from the registry); against a remote harness, whose
models are not known here, `/model <alias>` is sent as typed. A registered command at the
start of a line is highlighted in the composer and in sent messages (R27.6).

An A2UI ask (`channels/v1`, `kind: a2ui`) renders as a form in a rounded box: text by
variant, a choice picker as `(●)`/`( )` or `[x]`/`[ ]`, a text field as a box, a check
box, a slider as a bracketed gauge, a date-time input, tabs, a modal as a box under its
trigger, images, video and audio as their alt text. `tab` moves between inputs, `←→`
chooses, typing edits, `enter` on a button resolves the action's context against the data
model and sends it as an A2UI client `action` data part on the channel (`reply_to` the
ask). Every outgoing message declares `a2uiClientCapabilities` for the basic catalog.

Images (R28.2): an image artifact is decoded and drawn through `ratatui-image` when the
terminal answers the graphics query (kitty, iTerm2, sixel, half blocks); otherwise the
artifact's name and alt text are shown. `NO_COLOR` (or `TERM=dumb`) selects the
monochrome theme: the same glyphs, modifiers only.

## Current behaviour

- WHEN `rusty-harness` starts without `--no-tui` THEN it SHALL open the TUI as an A2A
  client of its own server; WHEN `--url` is given THEN it SHALL open the TUI against that
  harness and start no server (R27.3).
- WHEN the stream carries a `Task`, `TaskStatusUpdateEvent` or `TaskArtifactUpdateEvent`
  THEN the header, conversation and plan strip SHALL reflect it; nothing else is rendered
  (R25.3).
- WHEN the task is open THEN `^c` SHALL confirm before sending the complete transition,
  and `^x` SHALL send `CancelTask` (R27.4, R21.7).
- WHEN a message starts with a registered command THEN the token SHALL be highlighted
  (R27.6).
- WHEN a status message carries an A2UI payload THEN it SHALL render as a native form
  whose buttons send A2UI client actions on the channel (R25.4).
- WHEN an image artifact arrives THEN it SHALL be drawn where the terminal can, and shown
  as its name and alt text where it cannot (R28.2).
- WHEN `NO_COLOR` is set THEN every state SHALL render with the same glyphs and no colour
  (NFR accessibility).
- WHEN text is shown to a person THEN it SHALL come from the string catalog in the
  configured locale (R27.8, R25.6).

## Design

- [`design.md` § TUI renderer and CLI][design] (R27) and the prototype
  [`design/tui-renderer.html`](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design/tui-renderer.html).
- Code: `crates/rusty-harness/src/interaction/renderer/tui/`, `src/bin/rusty-harness.rs`.
- Snapshots: `crates/rusty-harness/tests/snapshots/tui_*.txt`, one per prototype state
  (`cargo nextest run -p rusty_harness --features tui -E 'test(tui_snapshot_)'`;
  `UPDATE_SNAPSHOTS=1` rewrites them).
- Not yet on the wire, so not yet rendered: tool-call boxes and streamed text deltas; the
  loop stores them but the stream carries replies on status updates. Push-to-talk arrives
  with voice (M6).

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M5, task 34) | The TUI: layout per the prototype, A2A client of the loopback or `--url`, controls, `/model` picker, command highlighting, native A2UI forms, images with the alt-text fallback, `NO_COLOR` | PR #11 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

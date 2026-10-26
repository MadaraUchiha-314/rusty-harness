# T4 — the demo end to end, `none` mode

<!-- markdownlint-disable MD013 -->

| Field | Value |
|---|---|
| Scenario | *The demo runs end to end in none mode* (`tests/e2e.rs::e2e_none_the_demo_runs_end_to_end`) |
| Command | `RH_E2E_EVIDENCE=… cargo nextest run -p rusty_harness --features cli -E 'test(e2e_none_)' --run-ignored all` |
| Configuration | [`tests/fixtures/demo/config.toml`](../../../../crates/rusty-harness/tests/fixtures/demo/config.toml): OpenAI `gpt-6.1-sol` (Responses API, key `env:OPENAI_API_KEY`), `durable.mode = "none"`, the file store, the [`release-notes`](../../../../crates/rusty-harness/tests/fixtures/demo/plugins/release-notes/) plugin |
| Outcome | **passed** (22.7 s), 2026-10-25 |
| Browser leg | `PW_VIDEO=1 RH_DEMO_URL=http://127.0.0.1:7474/ bunx playwright test --project=e2e demo.spec.ts` against the same config served by the binary: **passed** (11.7 s) |

## What ran (R32.1–3, R32.5)

One harness, two clients on it at once: the TUI (the real `App`, drawn on a test
terminal) and a second A2A client speaking the web renderer's protocol. One long-running
root task over three turns:

1. **The TUI** asks for release notes. The model calls `participant.ask` with an A2UI
   form from the basic catalog; the TUI draws it and the task waits, `input-required`,
   sub-state awaiting participant. The second client sees the same task waiting.
2. **The TUI** answers the form (`→ answer`); the model drafts the notes and the task
   waits for the next turn.
3. **The second client** asks for a shorter version; the model revises it; the task is
   still open. The TUI, given the task, shows the revision.

**Both renderers at once, recorded (R32.6):** the demo harness, the web renderer and the
TUI (served in a browser terminal by `ttyd`, attached with `--url`) side by side, each
working a task through the A2UI ask to the draft:
[`e2e/demo-both-renderers.webm`](e2e/demo-both-renderers.webm) (26 s), last frame
[`e2e/demo-both-renderers.png`](e2e/demo-both-renderers.png); spec
`web/tests/e2e/demo-recording.spec.ts`, passed (25.5 s).

The browser leg ran the same journey in the web renderer (the A2UI form through the
official React renderer): stills [`e2e/web-1-ask.png`](e2e/web-1-ask.png),
[`e2e/web-2-draft.png`](e2e/web-2-draft.png),
[`e2e/web-3-revised.png`](e2e/web-3-revised.png) and the recording
[`e2e/demo-web.webm`](e2e/demo-web.webm).

## Evidence

- TUI frames after each turn: [`e2e/console-none.md`](e2e/console-none.md)
- The task as it stands after turn 3: [`e2e/task-none.json`](e2e/task-none.json)
- The trace (OTLP export, flattened; local paths scrubbed): [`e2e/trace-none.json`](e2e/trace-none.json): 37 spans, one trace id

## Findings fixed on the way

- The web renderer showed the form's answer as "🎙 spoken message"; it now shows
  `→ answer`, as the TUI does (the browser leg asserts it).
- The trace recorded one `task.run` ending in `store i/o: … 00000004.json: entity not
  found`: the file store's directory listing deleted a write in flight (`*.tmp` before
  its rename). Listings no longer delete; the replay sweeps only stale temp files.

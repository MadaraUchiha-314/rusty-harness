# T5 — UI and visual

<!-- markdownlint-disable MD013 -->

The built web renderer against the locked prototypes, state by state, in the browser image CI uses (`mcr.microsoft.com/playwright:v1.64.1-noble`, through podman). Also the TUI's prototype states as `TestBackend` buffers. The screenshots compared against are the committed baselines under [`web/tests/visual/__screenshots__/`](../../../../web/tests/visual/__screenshots__/), and the TUI buffers are under [`crates/rusty-harness/tests/snapshots/`](../../../../crates/rusty-harness/tests/snapshots/). The multi-turn flow is recorded in [`e2e/demo-both-renderers.webm`](e2e/demo-both-renderers.webm).

## Playwright visual, phone and a11y projects (in the CI image)

```sh
bash scripts/visual-in-container.sh
```

Outcome: **passed** in 25s, 2026-10-25, at `0a97f5b`.

```text
✓   1 [phone] › tests/visual/phone.spec.ts:6:1 › streaming at phone width scrolls vertically only (1.5s)
  ✓   2 [visual] › tests/visual/states.spec.ts:21:1 › streaming: the reply is still arriving (2.2s)
  ✓   4 [visual] › tests/visual/states.spec.ts:30:1 › idle: input-required, the artifact attached, your turn (1.9s)
  ✓   5 [visual] › tests/visual/states.spec.ts:40:1 › a2ui: the ask renders as a form on the channel (2.3s)
  ✓   3 [a11y] › tests/a11y/axe.spec.ts:32:1 › the empty renderer and the streaming state have no AA violations (5.8s)
  ✓   6 [visual] › tests/visual/states.spec.ts:60:1 › command: the palette opens on / and the token is highlighted (2.0s)
  ✓   8 [visual] › tests/visual/states.spec.ts:70:1 › unknown: an unknown command is answered without the model (2.0s)
  ✓   9 [visual] › tests/visual/states.spec.ts:79:1 › done: complete is confirmed, then the controls close (2.2s)
  ✓  10 [visual] › tests/visual/states.spec.ts:96:1 › voice: listening, then the utterance is sent as 24 kHz PCM (2.9s)
  ✓   7 [a11y] › tests/a11y/axe.spec.ts:41:1 › idle, the palette and the confirmation have no AA violations (7.9s)
  ✓  11 [visual] › tests/visual/states.spec.ts:116:1 › tool and denied: the tool card states (2.6s)
  ✓  12 [a11y] › tests/a11y/axe.spec.ts:56:1 › the A2UI form has no AA violations (3.2s)
  ✓  13 [a11y] › tests/a11y/axe.spec.ts:64:1 › the keyboard flow: Enter, Shift+Enter, / and Esc (564ms)
  13 passed (23.1s)
```

## TUI prototype states

```sh
cargo nextest run -p rusty_harness --all-features -E 'test(tui_snapshot_)'
```

Outcome: **passed** in 1s, 2026-10-25, at `0a97f5b`.

```text
Starting 6 tests across 30 binaries (241 tests skipped)
        PASS [   0.043s] (1/6) rusty_harness::tui tui_snapshot_first
        PASS [   0.050s] (2/6) rusty_harness::tui tui_snapshot_chat
        PASS [   0.050s] (3/6) rusty_harness::tui tui_snapshot_done
        PASS [   0.055s] (4/6) rusty_harness::tui tui_snapshot_a2ui
        PASS [   0.022s] (5/6) rusty_harness::tui tui_snapshot_model
        PASS [   0.019s] (6/6) rusty_harness::tui tui_snapshot_voice
     Summary [   0.069s] 6 tests run: 6 passed, 241 skipped
```

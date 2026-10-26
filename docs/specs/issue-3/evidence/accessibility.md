# T9 — accessibility

<!-- markdownlint-disable MD013 -->

WCAG 2.1 AA on the web renderer: axe on every state in both themes, plus the keyboard flow. The TUI renders every state with `NO_COLOR`.

## axe (the a11y project of the same run)

```sh
bash scripts/visual-in-container.sh
```

Outcome: **passed** in 25s, 2026-10-25, at `0a97f5b`.

```text
✓   3 [a11y] › tests/a11y/axe.spec.ts:32:1 › the empty renderer and the streaming state have no AA violations (5.8s)
  ✓   7 [a11y] › tests/a11y/axe.spec.ts:41:1 › idle, the palette and the confirmation have no AA violations (7.9s)
  ✓  12 [a11y] › tests/a11y/axe.spec.ts:56:1 › the A2UI form has no AA violations (3.2s)
  ✓  13 [a11y] › tests/a11y/axe.spec.ts:64:1 › the keyboard flow: Enter, Shift+Enter, / and Esc (564ms)
  13 passed (23.1s)
```

## TUI states with NO_COLOR=1

```sh
NO_COLOR=1 cargo nextest run -p rusty_harness --all-features -E 'test(tui_snapshot_)'
```

Outcome: **passed** in 1s, 2026-10-25, at `0a97f5b`.

```text
Starting 6 tests across 30 binaries (241 tests skipped)
        PASS [   0.018s] (1/6) rusty_harness::tui tui_snapshot_first
        PASS [   0.020s] (2/6) rusty_harness::tui tui_snapshot_chat
        PASS [   0.020s] (3/6) rusty_harness::tui tui_snapshot_done
        PASS [   0.024s] (4/6) rusty_harness::tui tui_snapshot_a2ui
        PASS [   0.018s] (5/6) rusty_harness::tui tui_snapshot_voice
        PASS [   0.020s] (6/6) rusty_harness::tui tui_snapshot_model
     Summary [   0.039s] 6 tests run: 6 passed, 241 skipped
```

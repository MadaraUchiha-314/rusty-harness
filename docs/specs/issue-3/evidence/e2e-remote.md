# T4 — the demo end to end, `remote` mode (Temporal Cloud)

<!-- markdownlint-disable MD013 -->

| Field | Value |
|---|---|
| Scenario | *The demo runs end to end in remote mode* (`tests/e2e.rs::e2e_remote_the_demo_runs_end_to_end`) |
| Command | `RH_E2E_EVIDENCE=… cargo nextest run -p rusty_harness --features cli -E 'test(e2e_remote_)' --run-ignored all` |
| Configuration | [`tests/fixtures/demo/config.remote.toml`](../../../../crates/rusty-harness/tests/fixtures/demo/config.remote.toml): as `none`, with `durable.mode = "remote"` on Temporal Cloud: address and namespace by `file:~/.config/rusty-harness/…`, API key by `keyring:service=temporal,project=rusty-harness` (R32.4) |
| Outcome | **passed** (24.3 s), 2026-10-25, re-run at the verification node on `0a97f5b` (first run 20.3 s, task 46) |
| Workflow history | `scripts/demo-history.sh <task id> …`: 85 events (one workflow started, 12 activities, 3 signals, one per turn); the workflow is still running because the task stays open |

The same three turns as [the `none` run](e2e-none.md), with every phase of the loop a
Temporal activity on the operator's Cloud namespace.

## Evidence

- TUI frames after each turn: [`e2e/console-remote.md`](e2e/console-remote.md)
- The task after turn 3: [`e2e/task-remote.json`](e2e/task-remote.json)
- The trace: [`e2e/trace-remote.json`](e2e/trace-remote.json). The harness's own 37 spans form one trace, as in `none`; the other 165 are the Temporal SDK worker's own spans (polls, activations), exported by the same subscriber. Local paths scrubbed.
- The workflow history (payload blobs replaced by `<payload>`, the address and namespace by placeholders): [`e2e/history-remote.json`](e2e/history-remote.json)

## Findings fixed on the way

- `keyring:` only reached entries stored with `service` and `username`; the operator's key
  is stored with `project`. References now take attributes
  (`keyring:service=temporal,project=rusty-harness`), and `file:~/…` reads under `$HOME`.
- The Temporal client dialled `https` targets without TLS ("Connecting to HTTPS without
  TLS enabled"); an `https` target now uses TLS against the OS trust roots.

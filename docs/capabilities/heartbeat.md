# Capability: the heartbeat

> A task never silently stalls: a worker proves it is alive, re-signals what nobody
> picked up, and a hook decides what a stalled task deserves.

## What it is

`Heartbeat` ticks every `[loop] heartbeat_interval` (5 s by default). Each tick writes
the worker's liveness record (`liveness/<worker_id>`, with the loop's last boundary),
re-signals inbox events still `Received` or `Signalled` for longer than the interval,
and for every working task whose record has not changed within `[loop] stall_after`
runs the `task.stalled` hook. The hook answers `warn` (the default), `escalate` with a
reason, or `cancel`, which cancels the task through durable execution with a policy
reason. The binary starts the heartbeat with the server; the SDK exposes it for embedders.

## Current behaviour

- WHEN a tick runs THEN the system SHALL write the worker's liveness with the time of
  the loop's last boundary (R23.2).
- WHEN an inbox event's signal failed or was never acknowledged THEN the next tick SHALL
  signal it again, and a waiting task SHALL wake for it (R23.1).
- WHEN a working task has not progressed within `stall_after` THEN the system SHALL run
  `task.stalled` and act on its answer; the default only warns (R23.3).
- Ticks run under their own spans; they never borrow a task's trace.

## Design

- [`design.md` § Heartbeat and liveness][design] (R23).
- Code: `crates/rusty-harness/src/app_service/heartbeat/`.

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M3, task 24) | The heartbeat: liveness, re-signalling, the stall hook | [spec](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/), PR #9 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

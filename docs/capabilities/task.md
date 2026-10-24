# Capability: tasks, plans, state and persistence

> One task record aligned with A2A, a plan as a DAG, schema-checked state, and one
> atomic commit path into a memory or file store that survives a crash.

## What it is

`TaskRecord` carries what A2A does not (name, goal, acceptance criteria, mode,
participants with roles, sub- and parent-task links, the pending calls) and projects to
an A2A `Task` whose metadata holds those fields under the `task/v1` and
`participants/v1` extensions. The A2A `state` only ever holds standard values; what A2A
cannot say is a sub-state on the status message. `transition` is the one state machine.
`Plan` is a DAG of steps with inline topological ordering; `AgentState` holds JSON under
keys, validated against a declared schema. `PersistenceStore::commit` lands a task's
step atomically and idempotently; `MemoryStore` is the default and `FileStore` is a
journal from which every other record is rebuilt on open.

```mermaid
stateDiagram-v2
  [*] --> submitted
  submitted --> working: start
  working --> input_required: delivered (long-running) · ask · limit
  input_required --> working: resume
  working --> completed: delivered (one-shot) · complete
  input_required --> completed: complete
  working --> canceled: cancel
  input_required --> canceled: cancel
  working --> failed: fail
```

## Current behaviour

- WHEN a task changes state THEN the system SHALL accept only the moves of the diagram,
  run the `task.state_change` hooks and emit the A2A status update; an illegal move is a
  typed error that changes nothing.
- WHEN a task is exposed over A2A THEN its extra fields SHALL ride in `metadata` under
  the extension URIs and its sub-state on the status message.
- WHEN a plan edit would close a cycle THEN the system SHALL refuse it and leave the plan
  unchanged.
- WHEN a state write violates the key's schema THEN the system SHALL refuse it with the
  JSON pointer of the offending value.
- WHEN a commit carries events already in the log THEN the store SHALL skip them; WHEN
  the writer's expected version is stale THEN the store SHALL refuse the commit; WHEN
  the task is terminal THEN the store SHALL refuse further writes.
- WHEN the file store reopens after a crash THEN it SHALL replay its journal and rebuild
  any view the journal is ahead of.
- WHEN a subscriber tails a task THEN it SHALL see only the events its channels admit.

## Design

- [`design.md` § Task and lifecycle, § Plan and Step, § State and persistence][design]
  (R12–R14, R21).
- Code: `crates/rusty-harness/src/harness/core_loop/{task,plan,state,persistence}/`.

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M2, task 14) | Record and state machine, plan DAG, schema-checked state, memory and file stores, chaos test | [spec](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/), PR #8 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

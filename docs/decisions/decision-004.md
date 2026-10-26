# Decision 004: the workflow is started by the inbox; activities exchange store keys

- **Status:** accepted
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3)
- **Design:** [`docs/specs/issue-3/design.md`][design] § Durable execution; requirements
  R22.2, R22.4, R22.6

## Decision

One `TaskWorkflow` per task, with the task id as workflow id, is **started or signalled by
the inbox**: every event that reaches a task, a `SendMessage` from the server or a webhook
on `/inbox`, goes through `Inbox::accept`, which persists it and calls
`DurableExecution::start_or_signal`. The A2A executor never starts a workflow itself; it
streams the task's event log back to the caller.

The workflow's activities are the loop's four phases and they **exchange store keys, not
payloads**: `build_context` persists the request as `ctx/<task>/<n>`, `call_model` the
turn as `turn/<task>/<n>`, `execute_tool` one outcome as `tool/<task>/<n>/<call>`, and
`finish_iteration` reads them back. The in-process `none` mode runs the very same phases
in sequence.

## Why

- One ingestion path keeps the inbox's exactly-once guarantee (R20.5) and its
  participant guard in front of durable execution; a second start path would have to
  repeat both.
- Store keys keep a workflow history to ids, counts and outcomes: far below Temporal's
  payload limits, free of message content and of credentials (boundary 6), and small
  enough to replay cheaply.
- Sharing the phases between modes means the `none` mode and the Temporal modes cannot
  drift: a phase fixed once is fixed everywhere.

## Consequences

- Workers of one harness need one shared store; the file store replays the journal
  before reads and commits so several processes can share a directory.
- Retries are orchestrated by the workflow under the harness's own schedule, with the
  engine's attempts fixed at one, so the `durable.*` hook points see every failure.

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

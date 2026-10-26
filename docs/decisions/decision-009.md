# Decision 009: the model selection belongs to the task

- **Status:** accepted
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3)
- **Decider:** MadaraUchiha-314, on the work item
  ([comment](https://github.com/MadaraUchiha-314/rusty-harness/issues/3#issuecomment-6418742216))
- **Design:** [`docs/specs/issue-3/design.md`][design] § Models (R9.5), § Commands (R10)

## Decision

`/model` and `/effort` change the `ModelSelection` of the task they are sent in, and
nothing else:

- **In a task:** the change applies to that task's next model call. Every other task
  keeps its own selection. The task's record keeps it across a restart, and `/effort`
  reaches the request as `effort`.
- **Outside any task:** the change sets the default that new tasks start with. On the
  A2A server only the owner may run a command there.
- **Continuing a task:** a task that continues a finished one (`referenceTaskIds`)
  starts with that task's selection, which is how the TUI keeps a pick across
  follow-ups. A command sent with such a message acts on the continued task.

`ModelRouter` resolves a selection to a model instead of holding one shared selection.
Within the process its per-task copy wins over the record, and the next model call
writes it back, so a loop commit that read the record before the command cannot undo
the change.

## Why

The first build kept one selection for the whole process. On a server shared by
several people, a task's admin switched the model for every task. The design already
said "the task's `ModelSelection`" (§ Models); the code had not followed it.

## Consequences

- Changing the default (outside a task) is an owner's act. A participant's commands
  reach only their own tasks.
- The per-task copies live as long as the process. Workers in another process read the
  record, which a command writes at once.

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

# Capability: commands and the system prompt

> A person's `/command` never reaches the model; the system prompt is a plan of
> fragments a hook can rewrite.

## What it is

`CommandParser` reads `/<name>` from messages people write (never from tool results or
agent replies); `command.invoke` runs the command with the role check in the guard
stage; built-ins are `/exit`, `/model` and `/effort`; a plugin's declared command runs
on its endpoint. The system prompt is a `PromptPlan` (base, participants, extensions)
rendered from fragments through `prompt.assemble`; `[agent] system_prompt` replaces the
base with a file or a registered fragment.

## Current behaviour

- WHEN a person's message starts with `/<name>` THEN the system SHALL run the command
  and send nothing to the model; an unknown name is answered with a suggestion. The
  answer is an agent message with the command's line and, under `commands/v1`, the
  command token and its outcome (`done`, `exit`, `unknown`, `failed`); no task is opened,
  and only `/model` and `/effort` change the task, its model selection alone.
- WHEN a role may not invoke a command THEN the guard SHALL deny it and nothing changes.
- WHEN a command reaches the A2A server THEN it SHALL run as the sender's participant
  record on the task says (its roles there; a person only for a human participant), and
  outside a task only for the server's owner (its credential's principal, or the loopback
  operator); any other sender's `/text` SHALL be an ordinary message.
- WHEN `/model` or `/effort` runs in a task THEN that task's next model call SHALL use the
  selection, and every other task SHALL keep its own; the task's record keeps it across a
  restart.
- WHEN `/model` or `/effort` runs outside any task (only the owner may, on the server)
  THEN it SHALL change the default new tasks start with; a task that continues a finished
  one (`referenceTaskIds`) SHALL start with that task's selection.
- WHEN the system prompt is assembled THEN every text SHALL come from a fragment and
  `prompt.assemble` hooks SHALL see the plan before rendering.

## Design

- [`design.md` § System prompt, § Commands][design] (R7, R10).
- Code: `crates/rusty-harness/src/harness/{commands,system_prompt}/`.

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M2, task 13) | Prompt plan and hooks, the parser, built-ins, role guard, plugin commands | [spec](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/), PR #8 |
| issue-3 (M7, task 54) | Commands run on every path: `LocalAgent::send_message` answers a person's `/command` before the loop | PR #13 |
| issue-3 (M7, SR1-1) | The A2A server answers a `/command` before the message becomes an inbox event, so the TUI and web renderer get the same behaviour as the SDK (task 54 had covered only `send_message`) | PR #13 |
| issue-3 (M7, SR2-1) | Server-path commands run with the sender's roles from the task's participant record, not the forged Admin the server's caller carried | PR #13 |
| issue-3 (M7, operator review) | `/model` and `/effort` change only the task they are sent in (they had switched the model for every task in the process); outside a task they set the default; a follow-up task inherits the selection of the task it continues | PR #13 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

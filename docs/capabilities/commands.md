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
  and send nothing to the model; an unknown name is answered with a suggestion.
- WHEN a role may not invoke a command THEN the guard SHALL deny it and nothing changes.
- WHEN `/model` or `/effort` runs THEN the next model call SHALL use the selection.
- WHEN the system prompt is assembled THEN every text SHALL come from a fragment and
  `prompt.assemble` hooks SHALL see the plan before rendering.

## Design

- [`design.md` § System prompt, § Commands][design] (R7, R10).
- Code: `crates/rusty-harness/src/harness/{commands,system_prompt}/`.

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M2, task 13) | Prompt plan and hooks, the parser, built-ins, role guard, plugin commands | [spec](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/), PR #8 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

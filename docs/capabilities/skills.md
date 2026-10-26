# Capability: skills

> A `SKILL.md` with front matter is a skill; the model sees one line per skill until it
> activates one, and resources load only on demand.

## What it is

`SkillParser` reads the Agent Skills front matter (a name equal to the directory, a
description, optional compatibility, metadata and allowed tools) and the body. A
registered `Skill` holds the metadata only; `index_line()` is what the context window
lists at the skills position; `body()` reads the markdown once on activation and caches
it; `resource()` reads a file under the skill's root, never above it.

## Current behaviour

- WHEN a skill is registered THEN the system SHALL read its front matter only, and the
  body SHALL be read once, on first activation.
- WHEN a resource path escapes the skill root THEN the system SHALL refuse it.
- WHEN a skill's front matter violates the specification (name pattern, length limits,
  name not equal to the directory) THEN the system SHALL refuse it naming the field.
- WHEN a skill is activated THEN the system SHALL run the `skill.invoke` hooks and
  enter the body into the context as skill data, never as instructions.

## Design

- [`design.md` § Skills][design] (R6).
- Code: `crates/rusty-harness/src/harness/skills/`.

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M2, task 12) | Parser, progressive disclosure, activation | [spec](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/), PR #8 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

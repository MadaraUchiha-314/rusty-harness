# Capability: prompts and strings as assets

> Every prompt the harness sends a model is a markdown file; every string a person reads
> comes from one JSON catalog per locale.

## What it is

`prompts/**/*.md` (front matter: id, version, placeholders, description) are embedded at
build time and registered as `PromptFragment` entities. The only ways to build a
`Prompt` are `Prompt::fragment` (a registered file with its placeholders filled) and
`Prompt::data` (run-time data in a named role), so authored text cannot be a string
literal in code. `strings/en.json` is the source of truth for user-facing strings; the
build generates a `StringKey` constant per key, so a missing key does not compile, and
`Catalog::get` formats an ICU `MessageFormat` subset with per-key fallback to `en`.

## Current behaviour

- WHEN a prompt file's id does not match its path, or its placeholders differ from its
  body, THEN the build's registry SHALL refuse it (a contract test runs on every file).
- WHEN a fragment is rendered THEN every placeholder SHALL be filled and no argument
  SHALL be unknown; the prompt SHALL keep the fragment ids as provenance for the trace.
- WHEN a locale lacks a key THEN the system SHALL fall back to `en` for that key; a
  locale file whose keys or placeholders differ from `en` fails the tests.
- Patterns SHALL use `{name}`, `{n, plural, =0 {…} one {…} other {…}}` (with `#`) and
  `{x, select, a {…} other {…}}` only.
- A localized prompt SHALL be a sibling file `<id>.<locale>.md`.

## Design

- [`design.md` § Assets][design] (R33, Q18, Q19).
- Code: `crates/rusty-harness/src/assets/`, `crates/rusty-harness/build.rs`.

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M1, task 8) | Prompt fragments, the `Prompt` type, the string catalog and its generated keys | [spec](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/), PR #7 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

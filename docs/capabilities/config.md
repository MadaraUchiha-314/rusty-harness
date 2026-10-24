# Capability: configuration

> One typed configuration that the TOML file, the environment and the SDKs all produce,
> with credentials only ever by reference.

## What it is

`Config` mirrors `~/.rusty-harness/config.toml` section by section. `Config::load`
layers defaults ← file ← `RH__SECTION__KEY` environment variables ← code overrides and
deserializes once with unknown fields refused. The JSON schema is generated from the
types and published as `crates/rusty-harness/rusty-harness.schema.json`; the commented
defaults the first run writes are `config.default.toml`, kept equal to
`Config::default()` by a test.

## Current behaviour

- WHEN a value comes from code, the environment and the file THEN precedence SHALL be
  code, then environment, then file, then defaults.
- WHEN an environment variable `RH__A__B__C` is set THEN it SHALL set `a.b.c`, typed as
  TOML would type it (`true`, `42`, `["x"]`) and as a string otherwise.
- WHEN a field is unknown or of the wrong type THEN loading SHALL fail naming the field.
- WHEN a credential field holds a literal instead of `env:NAME`, `keyring:SERVICE/USER`
  or `file:PATH` THEN loading SHALL fail without echoing the value.
- Durations SHALL be written as `30s`, `15m`, `2h`, `500ms`.

## Design

- [`design.md` § Configuration and § Data models][design] (R27.2, R30.4, Q9).
- Code: `crates/rusty-harness/src/config/`.

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M1, task 7) | Typed `Config`, layered loading, the published schema, the default file | [spec](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/), PR #7 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

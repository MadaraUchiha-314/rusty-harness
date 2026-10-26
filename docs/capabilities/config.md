# Capability: configuration

> One typed configuration that the TOML file, the environment and the SDKs all produce,
> with credentials only ever by reference.

## What it is

`Config` mirrors `~/.rusty-harness/config.toml` section by section. `Config::load`
layers defaults ← file ← `RH__SECTION__KEY` environment variables ← code overrides and
deserializes once with unknown fields refused. The JSON schema is generated from the
types and published as `crates/rusty-harness/rusty-harness.schema.json`; the commented
defaults the first run writes are `config.default.toml`, kept equal to
`Config::default()` by a test. The same schema generates the SDKs' types: the npm
package's `Config` interface and the Python stub's `ConfigDict` `TypedDict`s, each
checked against it in CI.

## Current behaviour

- WHEN a value comes from code, the environment and the file THEN precedence SHALL be
  code, then environment, then file, then defaults.
- WHEN an environment variable `RH__A__B__C` is set THEN it SHALL set `a.b.c`, typed as
  TOML would type it (`true`, `42`, `["x"]`) and as a string otherwise.
- WHEN a field is unknown or of the wrong type THEN loading SHALL fail naming the field.
- WHEN a credential field holds a literal instead of `env:NAME`, `keyring:SERVICE/USER`,
  `keyring:ATTR=VALUE,…` or `file:PATH` THEN loading SHALL fail without echoing the value.
- WHEN a keyring entry is stored with attributes other than `service` and `username`
  THEN `keyring:service=temporal,project=rusty-harness` SHALL find it by those attributes
  (`secret-tool lookup service temporal project rusty-harness`), and a `file:~/…` path
  SHALL be read under the home directory.
- Durations SHALL be written as `30s`, `15m`, `2h`, `500ms`.

## Design

- [`design.md` § Configuration and § Data models][design] (R27.2, R30.4, Q9).
- Code: `crates/rusty-harness/src/config/`.

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M1, task 7) | Typed `Config`, layered loading, the published schema, the default file | [spec](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/), PR #7 |
| issue-3 (M7, tasks 42–43) | The schema generates the SDKs' types: `Config` for TypeScript (`bun run types`), `ConfigDict` and friends for Python (`stub_gen`); `Config.from_toml` / `from_dict` in Python, a `Config` object or TOML text in `Harness.create` | PR #13 |
| issue-3 (M7, task 46) | `keyring:ATTR=VALUE,…` and `file:~/…` references, for the demo's Temporal Cloud credentials | PR #13 |
| issue-3 (M7, SR1-10) | The Python `Config.from_toml` and `from_dict` apply the `RH__SECTION__KEY` layer (opt out with `env=False`); `Config::from_layers_json` takes the code layer as JSON | PR #13 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

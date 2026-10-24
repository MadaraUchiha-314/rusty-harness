# Decision 001: Repository tooling baseline

- **Status:** accepted
- **Work item:** [#2](https://github.com/MadaraUchiha-314/rusty-harness/issues/2)

## Decision

The workspace layout, toolchain and release pipeline are as described in
[tech stack](../development/tech-stack.md) and [CI and releases](../development/ci-and-releases.md).
The choices below are the ones the issue left open or that the dependency graph forced.

| Choice | Why |
|---|---|
| PyO3 family pinned to **0.29** (latest is 0.30) | `pyo3-stub-gen` 0.23 supports `pyo3 >=0.27, <0.30`. Bump together when it catches up. |
| MSRV **1.88** (not lower) | `pyo3-stub-gen` requires `time` 0.3.4x+, which needs rustc 1.88. The core crate alone builds on older compilers, but the MSRV is one workspace value. |
| `pyo3-stub-gen` licences and advisories allowed in `deny.toml` | Its `rustpython-parser` brings `CC0-1.0`, `Unicode-DFS-2016` and the unmaintained (not vulnerable) `unic-*` crates. Exceptions are scoped per crate and per advisory id. |
| release-plz publishes crates.io itself | Uses the token from `rust-lang/crates-io-auth-action` (trusted publishing), so tagging and publishing happen in one step and PyPI/npm fan out from the tag. |
| Python `[tool.uv] package = false` | The extension is built by `maturin develop`, not by `uv sync`, so `uv sync` stays fast and never rebuilds Rust. |
| No hand-written TS wrapper or `tsup` yet | `hello()`'s generated `.d.ts` is already `Promise<string>`; the wrapper lands with the first API that needs one. |
| `cdylib` crates skip `cargo test` | `rusty-harness-py` sets `test = false` so tests never link libpython; pytest covers it. wasm tests are `wasm32`-only and run under `wasm-pack test`. |
| JS/TS lint and format with **oxlint** (type-aware, via `oxlint-tsgolint`) and **oxfmt**, not eslint/prettier | Requested in review on #5. oxfmt also formats the repo's JSON, TOML and YAML. `tsc --noEmit` stays the type check. |

# Tech stack

| Layer | Crate / package | Tooling |
|---|---|---|
| Core | `crates/rusty-harness` → crates.io `rusty_harness` | Rust stable (edition 2024), `cargo`, `clippy`, `rustfmt`, `cargo nextest` |
| Python | `crates/rusty-harness-py` → PyPI `rusty_harness` | PyO3, `pyo3-async-runtimes` (tokio), maturin, `pyo3-stub-gen`, `uv`, `ruff`, `pyright`, `pytest` + `pytest-asyncio` |
| JS / TS | `crates/rusty-harness-wasm` → npm `rusty-harness` | `wasm-bindgen`, `wasm-pack`, `wasm-bindgen-test`, TypeScript (strict), `eslint`, `prettier`, `vitest` |
| Docs | `docs/` | VitePress, `markdownlint`, `cargo doc` for the API reference |
| Repo | root | `bun` (one JS toolchain), `pre-commit`/`prek`, `commitlint`, `cargo deny`, `cargo machete`, release-plz |

## Layout

```text
Cargo.toml              # the workspace: shared package metadata, deps and lint levels
rust-toolchain.toml     # stable + rustfmt, clippy, wasm32-unknown-unknown
crates/
  rusty-harness/        # core library — all logic lives here
  rusty-harness-py/     # PyO3 bindings — thin adapters only (publish = false)
  rusty-harness-wasm/   # wasm-bindgen bindings — thin adapters only (publish = false)
docs/                   # this site
```

Imports are `use rusty_harness::…` in Rust, `from rusty_harness import …` in Python and
`import { … } from "rusty-harness"` in TypeScript.

## Conventions

- **Lint levels** live once in the root `Cargo.toml` (`[workspace.lints.rust]`,
  `[workspace.lints.clippy]` — `clippy::all` and `clippy::pedantic` as warnings) and every
  crate inherits them with `[lints] workspace = true`. CI denies warnings.
- **MSRV** is `rust-version` in `[workspace.package]`. The workspace uses resolver 3, so
  Cargo prefers dependency versions that support it.
- **One version** for the whole workspace, in `[workspace.package]`. maturin reads it for
  the wheel and the npm build writes it into `package.json`.
- **Supply chain** — `deny.toml` holds the licence allowlist, advisory and source policy
  for `cargo deny`; `cargo machete` flags unused dependencies.

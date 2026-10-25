# Local development

## Prerequisites

| Tool | Install |
|---|---|
| Rust | [`rustup`](https://rustup.rs) — the toolchain itself comes from `rust-toolchain.toml` |
| cargo tools | `cargo install --locked cargo-nextest cargo-deny cargo-machete wasm-pack` (or `cargo binstall …`) |
| Python | [`uv`](https://docs.astral.sh/uv/) — it fetches the Python it needs |
| JavaScript | [`bun`](https://bun.sh) |
| Hooks | [`prek`](https://github.com/j178/prek) or `pre-commit` (`uv tool install pre-commit`) |

Install the JS dependencies for the whole repository once, from the root:

```sh
bun install
```

## Core crate (Rust)

```sh
cargo check --all-targets --all-features    # type check
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all --check
cargo nextest run --workspace --all-features
cargo test --doc --workspace --all-features
# The official TypeScript A2A SDK against the binary (builds it with --features cli):
bun run --cwd web test:contract
# The Temporal scenarios and the chaos suite start the SDK's dev server (the `temporal` CLI is
# downloaded once into ~/.cache/rusty-harness/temporal/):
cargo nextest run -p rusty_harness --all-features -E 'binary(durable_temporal) | binary(durable_chaos)'
# The TUI's prototype states as TestBackend buffers (UPDATE_SNAPSHOTS=1 rewrites tests/snapshots/tui_*.txt):
cargo nextest run -p rusty_harness --features tui -E 'test(tui_snapshot_)'
# The core must stay runtime-free for the browser:
cargo check --target wasm32-unknown-unknown -p rusty_harness --no-default-features --features rt-wasm
# Supply chain:
cargo deny check
cargo machete
```

## Python bindings

From `crates/rusty-harness-py/`:

```sh
uv sync                    # creates .venv with maturin, pytest, ruff, pyright
uv run maturin develop     # builds the extension into .venv; rerun after Rust changes
uv run pytest
uv run ruff check . && uv run ruff format --check .
uv run pyright
```

The type stub `rusty_harness.pyi` is generated from the Rust annotations. After changing
an exported function, regenerate it and commit the result — CI fails on a stale stub:

```sh
cargo run -p rusty-harness-py --bin stub_gen
```

## WASM and TypeScript bindings

From `crates/rusty-harness-wasm/`:

```sh
bun run build              # wasm-pack build for bundler, web and nodejs into dist/
bun run test               # vitest against the built package
bun run typecheck          # tsc --noEmit
wasm-pack test --node      # Rust-side wasm-bindgen tests
wasm-pack test --headless --chrome
```

From the root, `bun run lint` (oxlint, type-aware) and `bun run format` (oxfmt) cover every
TypeScript, JSON, TOML and YAML file; `bun run format:fix` rewrites them.

## Docs

```sh
bun run --cwd docs dev     # live preview at http://localhost:5173/rusty-harness/
bun run --cwd docs build
bun run lint:md
cargo doc --no-deps --all-features -p rusty_harness   # API reference
```

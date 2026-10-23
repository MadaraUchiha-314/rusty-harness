# Git hooks and commits

## Pre-commit hooks

`.pre-commit-config.yaml` runs the same checks CI runs. It works with both
[`prek`](https://github.com/j178/prek) (the Rust drop-in) and `pre-commit`. Install the
hooks once per clone:

```sh
prek install               # or: pre-commit install
```

The config installs both the `pre-commit` and the `commit-msg` hook. To run every hook
against the whole tree:

```sh
prek run --all-files
```

| Hook | What it runs |
|---|---|
| `cargo-fmt` | `cargo fmt --all --check` |
| `cargo-clippy` | `cargo clippy --all-targets --all-features -- -D warnings` |
| `cargo-nextest` | `cargo nextest run --workspace --all-features` |
| `cargo-check-wasm` | `cargo check` of the core for `wasm32-unknown-unknown` with `rt-wasm` |
| `ruff`, `ruff-format`, `pyright` | the Python crate |
| `eslint`, `tsc` | every TypeScript source |
| `markdownlint` | all Markdown |
| `commitlint` | the commit message (`commit-msg` stage) |

The hooks expect the [local development](./local-development) prerequisites, `bun install`
at the root and `uv sync` in `crates/rusty-harness-py/`.

## Commit messages

Commits follow [Conventional Commits](https://www.conventionalcommits.org), enforced by
`commitlint` with `@commitlint/config-conventional`:

```text
feat(core): add the agent loop
fix(py): release the GIL in hello_sync
chore: bump wasm-bindgen
```

The type matters: release-plz derives the next version and the changelog from it (`feat`
bumps the minor version, `fix` the patch, a `!` or `BREAKING CHANGE:` the major).

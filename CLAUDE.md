# CLAUDE.md

rusty-harness is a tiny, fast agent harness written in Rust. Work here runs through
[the-loop](https://github.com/MadaraUchiha-314/the-loop), enabled as a Claude Code plugin
in [`.claude/settings.json`](.claude/settings.json).

## How work is done

1. **Read the-loop's config first.** [`.the-loop/harness-config.yaml`](.the-loop/harness-config.yaml)
   is the project config; [`.the-loop/collaborators.yaml`](.the-loop/collaborators.yaml)
   lists who reviews and approves. The plugin's `the-loop` skill holds the operating rules.
2. **Every change is a work item with a GitHub issue.** Specs live in `docs/specs/<id>/`;
   scale the rigor to the change per the skill's risk tiers.
3. **Keep the knowledge trees current** in the same PR as the change:
   `docs/architecture/`, `docs/capabilities/`, `docs/decisions/`, `docs/learnings/`.

## Tooling

The layout, toolchain and every local command are documented in
[`docs/development/`](docs/development/local-development.md). Install the git hooks once
(`prek install`); they run the same checks as CI. Before pushing, at minimum:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --workspace --all-features && cargo test --doc --workspace --all-features
cargo check --target wasm32-unknown-unknown -p rusty_harness --no-default-features --features rt-wasm
```

All logic lives in `crates/rusty-harness`; `crates/rusty-harness-py` and
`crates/rusty-harness-wasm` are thin adapters. Commits are Conventional Commits.

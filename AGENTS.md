# AGENTS.md

rusty-harness is a tiny, fast agent harness written in Rust, built through
[the-loop](https://github.com/MadaraUchiha-314/the-loop).

Read `.the-loop/harness-config.yaml` and the-loop skill before implementing a work item.
Every change is a work item with a GitHub issue; its specs and evidence live under
`docs/specs/<id>/`. Update the affected docs under `docs/` with the code. Before pushing,
run `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` and
`cargo test`. Explicit user instructions and the work item's frozen selections govern
scope and approvals.

# AGENTS.md

rusty-harness is a tiny, fast agent harness written in Rust, built through
[the-loop](https://github.com/MadaraUchiha-314/the-loop).

Read `.the-loop/harness-config.yaml` and the-loop skill before implementing a work item.
Every change is a work item with a GitHub issue; its specs and evidence live under
`docs/specs/<id>/`. Update the affected docs under `docs/` with the code. Before pushing,
run the git hooks (`prek run --all-files`) — the command list is in
`docs/development/local-development.md`. All logic lives in `crates/rusty-harness`; the
`-py` and `-wasm` crates are thin adapters. Commits are Conventional Commits. Explicit
user instructions and the work item's frozen selections govern scope and approvals.

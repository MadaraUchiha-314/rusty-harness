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

Use Cargo's own commands, and run all of them before pushing:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

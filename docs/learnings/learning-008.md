# Learning 008: on a small machine, the build is the bottleneck: cap jobs, prune incremental

- **Date:** 2026-10-25
- **Source:** system-feedback
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3) (M5–M7)
- **Category:** tooling

## What happened

The workspace builds with every feature (Temporal, the TUI, audio, MCP, the server)
for a native target, wasm32 and Python. On a 7.5 GB machine, cargo's default
parallelism made the linker swap until the session stalled. Later
`target/debug/incremental` grew to 210 GB and filled the disk. The only symptom was
a linker "signal 7 (Bus error)" that looked like a compiler crash.

## Learning

- Build with `CARGO_BUILD_JOBS=2` on a machine under about 16 GB.
- When a linker or rustc dies with SIGBUS, check `df` first, then delete
  `target/debug/incremental`. That frees the space and costs one slower rebuild.
- A nextest `-E` filter still builds every test binary, so narrow the build with
  `-p` and `--test` instead.

## Applied in

[`docs/development/local-development.md`](../development/local-development.md) § Build
resources.

# Learning 006: on wasm32, `std::time::Instant::now` compiles and then panics

- **Date:** 2026-10-25
- **Source:** system-feedback
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3) (M7, task 42)
- **Category:** platform

## What happened

The core crate had checked cleanly for `wasm32-unknown-unknown` since M1. Task 42 was
the first to run a harness turn there (the `wasm-pack` scenario), and the core's
timing went through `std::time::Instant`. On that target, std provides the type but
`Instant::now` panics at run time ("time not implemented on this platform"), and
`cargo check` cannot see it.

## Learning

A clean wasm32 `cargo check` proves only that the code compiles. Time, randomness,
threads and the filesystem must go through the runtime seam: `runtime::Instant`
(`web-time`, which uses `performance.now()` in the browser), the host's clock and RNG.
Prove the browser build by running it (`wasm-pack test --node` and `--headless
--chrome`), not only by checking it.

## Applied in

`crates/rusty-harness/src/runtime.rs` (`pub use web_time::Instant`),
`crates/rusty-harness-wasm/tests/scenario/`, [capability: browser](../capabilities/browser.md).

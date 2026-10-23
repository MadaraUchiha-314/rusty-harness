# Architecture

The architecture index for rusty-harness. Add one section per component as it lands,
linking its design under `docs/specs/<id>/design.md`.

## Workspace

One Cargo workspace, three crates, one version
([tech stack](../development/tech-stack.md)):

| Crate | Role | Ships as |
|---|---|---|
| `crates/rusty-harness` | All logic. Async, runtime-agnostic. | crates.io `rusty_harness` |
| `crates/rusty-harness-py` | PyO3 adapter: awaitable + `*_sync` per async API. | PyPI `rusty_harness` |
| `crates/rusty-harness-wasm` | wasm-bindgen adapter: `Promise` per async API. | npm `rusty-harness` |

## Runtime seam

`rusty_harness::runtime::Runtime` is the only way the core spawns or waits. Implementations
are feature-gated — `rt-tokio` (`TokioRuntime`) and `rt-wasm` (`WasmRuntime`, `wasm32`
only) — and there is no default. `MaybeSend` stands in for `Send` so the same bounds hold
on `wasm32`. See the [async model](../guide/async-model.md) and
[decision-001](../decisions/decision-001.md).

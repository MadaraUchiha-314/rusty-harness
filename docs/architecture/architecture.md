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

## Layers and cross-cutting modules (issue-3)

The core crate's module tree is the architecture diagram of
[issue #3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3): one top-level
module per layer, one sub-module per box, plus five cross-cutting modules that belong to
no layer. A contract test keeps the tree equal to the one
[`design.md`](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md)
draws.

| Module | Role | Capability doc |
|---|---|---|
| `interaction/` | layer 1: surfaces, renderers, inbound events | [surfaces and renderers](../capabilities/surfaces.md), [the terminal renderer](../capabilities/tui.md), [inbox](../capabilities/inbox.md) (events) |
| `app_service/` | layer 2: A2A server, inbox, channels API, heartbeat, durable execution, o11y | [server](../capabilities/server.md), [inbox](../capabilities/inbox.md), [heartbeat](../capabilities/heartbeat.md), [durable execution](../capabilities/durable.md), [loop](../capabilities/loop.md), [observability](../capabilities/observability.md) |
| `harness/` | layer 3: the core loop and its entities, prompts, skills, tools, hooks, models, A2A client, channels, plugins, commands | [loop](../capabilities/loop.md), [task](../capabilities/task.md), [models](../capabilities/models.md), [tools](../capabilities/tools.md), [plugins](../capabilities/plugins.md), [skills](../capabilities/skills.md), [channels](../capabilities/channels.md), [commands](../capabilities/commands.md), [hooks](../capabilities/hooks.md) |
| `self_improvement/` | layer 4: placeholders, out of scope | — |
| `registry/` | entity identity and resolution | [registry](../capabilities/registry.md) |
| `host/` | the platform behind typed capabilities (HTTP, clock, secrets, files, processes, audio, log sink); native defaults, host-supplied on `wasm32` | — |
| `config/` | typed configuration, layered loading, published schema | [config](../capabilities/config.md) |
| `assets/` | prompt fragments and the string catalog, embedded by `build.rs` | [assets](../capabilities/assets.md) |
| `error.rs` | `HarnessError` and the one mapping to A2A error codes | — |

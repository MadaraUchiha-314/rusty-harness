---
type: evidence
workItem: "github:MadaraUchiha-314/rusty-harness#3"
---

# Documentation: rusty-harness, the agent harness (issue #3)

<!-- markdownlint-disable MD013 -->

Issue #3 built the product, so every capability doc below was created by it. Each has a
history row per milestone that changed it, pointing at the PR of that milestone.

## Capability docs

| Capability doc | What changed (its history rows, by milestone) | Pull requests |
|----------------|--------------|-------------|
| [entities and the registry](../../../capabilities/registry.md) | M1, task 3: Introduced `Entity`, `EntityId`, `EntityKind`, `Placement`, `Provenance` and `Registry` | PR #7 |
| [hooks on the whole lifecycle](../../../capabilities/hooks.md) | M1, tasks 5–6: Catalogue, dispatcher with the guard stage, remote hooks | PR #7 |
| [configuration](../../../capabilities/config.md) | M1, task 7: Typed `Config`, layered loading, the published schema, the default file; M7, tasks 42–43: The schema generates the SDKs' types: `Config` for TypeScript (`bun run types`), `ConfigDict` and friends for Python (`stub_gen`); `Config.from_toml` / `from_dict` in Python, a `Config` object or TOML text in `Harness.create`; M7, task 46: `keyring:ATTR=VALUE,…` and `file:~/…` references, for the demo's Temporal Cloud credentials; M7, SR1-10: the Python Config applies the environment layer | PR #7, PR #13 |
| [prompts and strings as assets](../../../capabilities/assets.md) | M1, task 8: Prompt fragments, the `Prompt` type, the string catalog and its generated keys | PR #7 |
| [the host](../../../capabilities/host.md) | M1, task 4: `Host`, the capability traits, the native and testing hosts, secrets; M6, tasks 39–40: Native WebSocket, cpal microphone and speaker; M7, task 46: `keyring:ATTR=VALUE,…` and `file:~/…` references; M7, task 42: The host built from page callbacks on `wasm32`; M7, SR1-15, SR1-16: keyring attributes with `/`, `~` without HOME refused | PR #7, PR #12, PR #13 |
| [models, LLM and System One](../../../capabilities/models.md) | M2, tasks 11, 13: `Llm`, OpenAI and Anthropic clients, `Retrying`, System One, the model router behind `/model` and `/effort`; M6, task 40: Spoken turns: transcription before the model, synthesis of the reply; M6, task 39: OpenAI `Voice`, `SpeechToText`, `TextToSpeech`; the native WebSocket; the harness-side relay and `voice/v1` parts; M6, task 38: Image and audio parts from messages into the context; the up-front modality refusal; M7, task 45: The OpenAI Responses wire and `api`; provider-safe tool names; usage and the cache hit rate measured live; M7, SR1-7, SR1-11..14: strict model streams and wire names; M7, operator review: `/model` and `/effort` change only the task they are sent in | PR #8, PR #12, PR #13 |
| [agent plugins](../../../capabilities/plugins.md) | M2, tasks 12–13, 15: Discovery, validation, all-or-nothing install, declared hooks and commands, MCP servers from `mcp.json`; M7, task 51: An unlistable configured plugin path fails the build instead of being skipped | PR #8, PR #13 |
| [skills](../../../capabilities/skills.md) | M2, task 12: Parser, progressive disclosure, activation | PR #8 |
| [tools and MCP](../../../capabilities/tools.md) | M2, task 15: Executor, policy guard, size policy, MCP over `rmcp` and the host channel; M6, task 38: `ToolOutput::Artifacts`: images and files for the participants | PR #8, PR #12 |
| [tasks, plans, state and persistence](../../../capabilities/task.md) | M2, task 14: Record and state machine, plan DAG, schema-checked state, memory and file stores, chaos test; M7, task 53: The `plan.update` and `plan.step_done` tools, the `step.execute` operation, the plan to the context and to surfaces as a `plan/v1` artifact; M7, SR1-2: plan edits serialised per task, one temp file per write | PR #8, PR #13 |
| [the core loop](../../../capabilities/loop.md) | M2, tasks 16–17: Context builder and compactor, the agent trait and the loop, durable mode `none` | PR #8 |
| [sub-agents, participants and channels](../../../capabilities/channels.md) | M2, task 18: Remote, loopback and browser transports, `agent.call`, channels and the participant tools, extension schemas; M3, task 23: Channels on the server: the per-viewer projection, push to allowlisted hosts; M5, task 33: `participant.ask` with `kind: a2ui` validates the payload against the basic catalog and emits the text alternative with the typed data part | PR #8, PR #9, PR #11 |
| [commands and the system prompt](../../../capabilities/commands.md) | M2, task 13: Prompt plan and hooks, the parser, built-ins, role guard, plugin commands; M7, task 54: Commands run on every path: `LocalAgent::send_message` answers a person's `/command` before the loop; M7, SR1-1, SR2-1: commands answered on the server path, with the sender's roles from the task's participant record; M7, operator review: `/model` and `/effort` change only the task they are sent in | PR #8, PR #13 |
| [the A2A server](../../../capabilities/server.md) | M3, tasks 20–21, 23, 25: Server on the official SDK, the protocol gap, channels on the server, the `cli` binary; M3, task 27: The `/inbox` route, the conformance suite in CI; M5, task 33: The card advertises the basic A2UI catalog; each stream is negotiated for the surface that opened it; M5, task 37: `[server] allow_origins` (CORS for browser surfaces); the card lists commands, model aliases and `application/a2ui+json` among its modes; a stream starts after the last event its viewer can see and projects each event with the caller's current memberships; M7, SR2-1, CR1-1, CR1-2: command roles from the task, one participation rule, redacted cancel | PR #9, PR #11, PR #13 |
| [the durable inbox](../../../capabilities/inbox.md) | M3, task 22: The inbox, its tri-state events, the delivery policy, the webhook adapters; M3, task 27: The `/inbox` route behind the gate and the participant guard | PR #9 |
| [the heartbeat](../../../capabilities/heartbeat.md) | M3, task 24: The heartbeat: liveness, re-signalling, the stall hook | PR #9 |
| [durable execution](../../../capabilities/durable.md) | M2, task 15: The `none` mode: the loop in process under the retry policy; M4, tasks 28–31: Temporal client, worker and dev server; the task workflow and its activities; retry orchestration, timeouts and crash recovery; outbound reconciliation; M7, SR1-8: API key only over TLS | PR #8, PR #10, PR #13 |
| [observability](../../../capabilities/observability.md) | M1, task 9: Tracing hooks, OTLP export, the redaction layer; M3, task 26: One persisted trace per task, Langfuse hook set, the end-to-end canary | PR #7, PR #9 |
| [surfaces and renderers](../../../capabilities/surfaces.md) | M5, task 33: `SurfaceCapabilities` and negotiation, the `Renderer` trait, the A2UI 0.9.1 model, typed ask payloads, the card's catalog advertisement | PR #11 |
| [the terminal renderer](../../../capabilities/tui.md) | M5, task 34: The TUI: layout per the prototype, A2A client of the loopback or `--url`, controls, `/model` picker, command highlighting, native A2UI forms, images with the alt-text fallback, `NO_COLOR`; M5, task 37: A remote card supplies the agent name, the commands and the model aliases; the one-harness A2UI scenario (`tests/tui.rs::on_one_harness`); M6, task 40: Push-to-talk (hold and toggle) on `cpal`, spoken-reply playback, `tui_snapshot_voice`; M7, SR1-25: wire payloads decoded as doubles | PR #11, PR #12, PR #13 |
| [the web renderer](../../../capabilities/web.md) | M5, task 35: The web renderer package: AI Elements copied in, the SDK client, the per-task store, A2UI through `@a2ui/react`, the composer with the palette, the task panel, strings from `en.json`; M5, task 36: The visual contract on Playwright against a scripted mock, axe at AA in both themes, the keyboard flow, the bundle scan, the capture script; M5, task 37: The end-to-end leg against the real binary (`test:e2e`); M6, tasks 38, 40: Image artifacts from raw bytes; the voice button and spoken-reply playback; the `Buffer` polyfill | PR #11, PR #12 |
| [multi-modal input and output, and voice](../../../capabilities/voice.md) | M6, tasks 38–41: Image and audio parts, tool artifacts, the OpenAI speech providers, the relay, spoken turns, voice on the TUI and the web | PR #12 |
| [the harness in the browser (WebAssembly)](../../../capabilities/browser.md) | M7, task 42: The npm SDK: `Harness.create`, `sendMessage`, `cancelTask`, `subscribe`, `getTask`; the JS host; generated `Config` types; M7, SR1-5, SR1-6, SR1-20, CR1-3: redirects fail under allowOrigins, no eval, abort on drop, follow-up context | PR #13 |

The index, [`capabilities.md`](../../../capabilities/capabilities.md), lists all 23, one
row each, ordered along the architecture diagram's modules.

## Documentation

| Document | What changed |
|----------|--------------|
| [`README.md`](../../../../README.md) | Rewritten: what the harness is, the four ways to install it (the command, Rust, Python, JS), a quick start, the demo |
| [Docs home](../../../index.md) | The features and the calls to action describe the product (surfaces, durable tasks, extensibility, three ecosystems) |
| [Architecture](../../../architecture/architecture.md) | One section per layer and for the cross-cutting modules, the layer and runtime diagrams, packaging |
| [Getting started](../../../guide/getting-started.md) | The `rusty-harness` command and its installs; Rust, Python and TypeScript examples that run a real turn |
| [Embed the SDK](../../../guide/embed-the-sdk.md) | The SDK verbs, the Python and browser sections |
| [Run the server](../../../guide/run-the-server.md) | Created in M3; credential reference forms |
| [Run durably](../../../guide/run-durably.md) | Created in M4 |
| [Run the TUI](../../../guide/run-the-tui.md) | Created in M5; voice in M6 |
| [Run the web renderer](../../../guide/run-the-web-renderer.md) | Created in M6 |
| [Run the demo](../../../guide/run-the-demo.md) | Created in M7: both modes, the tests, the recording, the history script |
| [Write a plugin](../../../guide/write-a-plugin.md) | Created in M2 |
| [Local development](../../../development/local-development.md) | Every layer's commands; packaging, measurement, the cache hit rate, the audio prerequisite |
| [Decisions](../../../decisions/decisions.md) | 001–006 |
| [`crates/rusty-harness-wasm/README.md`](../../../../crates/rusty-harness-wasm/README.md), [`crates/rusty-harness-py/README.md`](../../../../crates/rusty-harness-py/README.md), [`packaging/npm/README.md`](../../../../packaging/npm/README.md), [`packaging/pypi-cli/README.md`](../../../../packaging/pypi-cli/README.md) | The package pages of npm and PyPI: the SDKs and the binary |

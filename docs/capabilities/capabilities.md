# Capabilities

The organized view of the specs: one `<capability>.md` per capability, describing current
behaviour. Add one line per capability doc here.

| Capability | One line |
|---|---|
| [Entities and the registry](registry.md) | one identity and one trait per pluggable unit; resolve or replace by `(id, version)` |
| [Hooks on the whole lifecycle](hooks.md) | pre, guard, in and post stages on 40 catalogue points, local or remote |
| [Configuration](config.md) | one typed `Config` from file, environment and code; credentials by reference |
| [Prompts and strings as assets](assets.md) | prompts are markdown files, strings come from one catalog per locale |
| [Models, LLM and System One](models.md) | one `Llm` over every provider, retried and modality-checked; `/model` switches the next call |
| [Agent plugins](plugins.md) | a plugin directory contributes skills, servers, hooks, commands and prompts, all or nothing |
| [Skills](skills.md) | `SKILL.md` metadata at startup, body on activation, resources on demand |
| [Tools and MCP](tools.md) | concurrent calls in issue order, policed by role, sized before the context; MCP under `rmcp` |
| [Tasks, plans, state and persistence](task.md) | one A2A-aligned record, a plan DAG, schema-checked state, an atomic commit journal |
| [The core loop](loop.md) | sherma's agent in A2A types, four hookable steps, limits, context positions and compaction |
| [Sub-agents, participants and channels](channels.md) | every sub-agent over A2A through one tool; asks through owned channels |
| [Commands and the system prompt](commands.md) | `/commands` from people only; the prompt is a plan of fragments |
| [The A2A server](server.md) | A2A 1.0 on the official SDK behind a gate: auth, participants, limits, extensions; loopback without credentials |
| [The durable inbox](inbox.md) | one door for every event, stored before answered, applied exactly once; `POST /inbox` for webhooks |
| [The heartbeat](heartbeat.md) | liveness per worker, re-signalled events, a hook on stalled tasks |
| [Durable execution](durable.md) | three modes behind one interface; on Temporal every phase is an activity, retries are the workflow's, a killed worker is picked up |
| [Observability](observability.md) | one persisted trace per task over OTLP; Langfuse as hooks; logs without credentials |
| [Surfaces and renderers](surfaces.md) | a surface declares what it renders and gets text for the rest; A2UI 0.9.1 from the basic catalog, typed |
| [The terminal renderer](tui.md) | one command opens a TUI that is an A2A client of the harness it started or of `--url`; native A2UI forms, images, `NO_COLOR` |

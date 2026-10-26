# Capability: the harness in the browser (WebAssembly)

> The whole harness runs in a page: the loop, the registry, hooks, skills and `none`
> durability, with every platform capability handed in by the page as a callback.

## What it is

`rusty_harness` builds for `wasm32-unknown-unknown` with `--no-default-features
--features rt-wasm`, and CI keeps tokio out of that dependency tree (R29.5). The npm
package `rusty-harness` (crate `rusty-harness-wasm`) wraps it in one class:

```ts
import { Harness } from "rusty-harness";

const harness = await Harness.create(config, host); // Config object or TOML text
for await (const event of harness.sendMessage({ message })) render(event);
await harness.cancelTask(taskId);
for await (const event of harness.subscribe(taskId)) render(event);
```

`Config` is generated from `rusty-harness.schema.json` (`bun run types`), so a typed
object and the TOML file describe the same thing (R30.3). Events are the A2A 1.0 JSON
binding: `{ task }`, `{ message }`, `{ statusUpdate }` or `{ artifactUpdate }`. A failed
call rejects with an `Error` carrying the A2A JSON-RPC `code`.

The page's `HostOptions` supply the capabilities of
[the host table](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md#host-capabilities-r47-r29):
`fetch` (default `globalThis.fetch`, restricted to `allowOrigins` when given),
`webSocket` (default `new WebSocket(url)`; a browser cannot send headers, so a socket
that needs one is refused), `now` (`Date.now`), `randomBytes`
(`crypto.getRandomValues`), `log` (`console`) and `secret`, which has no default: the
page resolves each configured reference (`env:OPENAI_API_KEY`) itself, or the harness
does not build.

## Current behaviour

- WHEN the page creates a harness THEN the system SHALL build it from the same `Config`
  the CLI uses and the page's host, failing closed when a configured credential has no
  resolver (R29.3).
- WHEN `sendMessage` is called THEN the turn SHALL stream back as an `AsyncIterable` of
  A2A events, driven on the page's event loop, and run to its end even when the page
  stops reading.
- WHEN a model call targets an origin outside `allowOrigins` THEN the host SHALL refuse
  it before `fetch` is called.
- WHEN the core needs time on `wasm32` THEN it SHALL read `performance.now()` and
  `Date.now()`, never the native clock (which panics there).
- WHEN the host has `allowOrigins` THEN a fetch SHALL fail on a redirect rather than
  follow it to another origin.
- WHEN a page's CSP forbids `unsafe-eval` THEN the SDK SHALL still create a harness and
  run a turn.

### Unavailable in the browser

This is the whole list (R29.3); everything else is a capability the page can supply.

| Feature | Why | How it fails |
|---|---|---|
| File plugins, the `file` store, `--config` | no file system | `Unsupported(FileSystem)` at construction |
| stdio MCP servers | no processes | `Unsupported(ProcessSpawner)` at construction |
| `embedded` and `remote` durability | the Temporal SDK is native-only | `durable-temporal` does not compile for `wasm32` |
| The TUI and the A2A server | terminals and sockets | not compiled for `wasm32` |
| `env:`, `keyring:`, `file:` resolved by the harness | no environment | the page's `secret` callback resolves them |

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M7, task 42) | The npm SDK: `Harness.create`, `sendMessage`, `cancelTask`, `subscribe`, `getTask`; the JS host; generated `Config` types | PR #13 |
| issue-3 (M7, SR1-5, SR1-6, SR1-20, CR1-3) | With `allowOrigins` a redirect fails the request; the SDK compiles no code from strings (strict CSP, a CI leg without code generation); a dropped fetch is aborted and a dropped socket closed; a follow-up naming only its task takes that task's context | PR #13 |

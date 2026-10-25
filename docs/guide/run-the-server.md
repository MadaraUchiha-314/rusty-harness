# Run the server

The `rusty-harness` binary is the harness as a user runs it: it starts the agent, serves
it over A2A 1.0 and opens the [terminal UI](./run-the-tui) (`--no-tui` keeps it a
server). It is the same `HarnessBuilder` the [SDK guide](./embed-the-sdk) assembles, from
the same configuration.

## Install and first run

```sh
cargo install rusty_harness --features cli --bin rusty-harness
rusty-harness --no-tui
```

The first run writes `~/.rusty-harness/config.toml` with every default commented, then
tells you what it did:

```text
wrote ~/.rusty-harness/config.toml with commented defaults
durable mode none: no Temporal configured; tasks will not survive a restart
loopback only: no security scheme configured
agent card: http://127.0.0.1:7474/.well-known/agent-card.json
harness listening on http://127.0.0.1:7474/
```

Give it a model before the first conversation. Credentials are references, never values:

```toml
[models.default]
provider = "openai"            # openai | anthropic
model = "gpt-6.1"
api_key = "env:OPENAI_API_KEY" # env:NAME, keyring:SERVICE/USER, keyring:ATTR=VALUE,… or file:PATH
```

Every reference is resolved at startup; one that does not resolve stops the process with
exit code 2 and names the reference, never the value.

From a checkout, the demo configuration under `tests/fixtures/demo/` is the same thing
with a plugin directory and a file store:

```sh
cargo run -p rusty_harness --features cli -- \
  --config crates/rusty-harness/tests/fixtures/demo/config.toml --no-tui
```

## Flags

| Flag | Meaning |
|---|---|
| `--config <path>` | The configuration file; written with commented defaults when missing. Precedence: this flag, then `~/.rusty-harness/config.toml`, then the built-in defaults. `RH__SECTION__KEY` environment variables override any file. |
| `--bind <addr>` | Bind here instead of `[server] bind`. |
| `--no-tui` | No terminal UI: log to standard output. |
| `--locale <tag>` | The locale of everything a person reads: this flag, then `[locale]`, then `LANG`, then `en`. |
| `--url <a2a-url>` | Talk to a running harness instead of starting one (the terminal UI, milestone 5). |

## Talk to it

Any A2A 1.0 client works. With the official TypeScript SDK:

```ts
import { ClientFactory } from "@a2a-js/sdk/client";

const client = await new ClientFactory().createFromUrl("http://127.0.0.1:7474/");
const sent = await client.sendMessage({
  message: { messageId: crypto.randomUUID(), role: "ROLE_USER", parts: [{ text: "hello" }] },
});
```

Or JSON-RPC by hand. Requests must name the protocol version:

```sh
curl -s http://127.0.0.1:7474/ -H 'A2A-Version: 1.0' -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"SendMessage","params":{"message":{"messageId":"m1","role":"ROLE_USER","parts":[{"text":"hello"}]}}}'
```

A task is one-shot unless the first message's metadata says otherwise under the
[`task/v1` extension](../a2a/ext/task/v1): `{"mode": "long-running"}` keeps it in
`input-required` between turns. Webhooks and other agents post events to `/inbox` with
the same credentials; see [the durable inbox](../capabilities/inbox.md).

## Reach it from elsewhere

Without `[server.security]` the server binds loopback only and refuses any other
address. To listen on a network interface, configure a credential and let clients
present it:

```toml
[server]
bind = "0.0.0.0:7474"
public_url = "https://agent.example.com"

[server.security]
scheme = "bearer"        # bearer | api-key
token = "env:RH_BEARER"

[server.limits]
requests_per_minute = 120
streams_per_caller = 8

[server.push]
allow_hosts = ["hooks.example.com"]   # push-notification webhooks go nowhere else
```

A client acting for a named participant sends `x-rusty-harness-participant: <id>`; a task
is visible to its participants only. Everything the gate enforces is listed in
[the A2A server](../capabilities/server.md).

## Observe it

```toml
[o11y]
log_level = "info"
otlp_endpoint = "http://127.0.0.1:4318"   # OTLP/HTTP; /v1/traces is appended
langfuse = false                          # true adds Langfuse's observation attributes
```

Every operation of a task lands in one trace, from the request that created it to its
terminal state; a request may bring its own `traceparent`. Logs never carry a
credential, and content is truncated above `debug`. See
[observability](../capabilities/observability.md).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Stopped by Ctrl-C. |
| 1 | The server stopped with an error. |
| 2 | The configuration could not be loaded or written, a credential reference did not resolve, the durable mode cannot run in this build, or the harness could not start. |

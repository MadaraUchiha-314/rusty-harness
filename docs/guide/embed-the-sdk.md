# Embed the SDK

The harness is a library first. `HarnessBuilder` assembles the same pieces the CLI
uses, from the same `Config`, and hands you an `Agent` you drive in process with no
server, no Temporal and no renderer.

## A harness in a test

```rust
use std::sync::Arc;

use rusty_harness::config::Config;
use rusty_harness::harness::core_loop::agent::{Agent, AgentRequest, CallContext};
use rusty_harness::harness::core_loop::task::{Caller, Role};
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{ModelEvent, StopReason};
use rusty_harness::host::Host;
use rusty_harness::HarnessBuilder;
use futures::StreamExt as _;

# #[tokio::main(flavor = "current_thread")]
# async fn main() {
let scripted = RecordedLlm::new(
    "default",
    vec![vec![
        ModelEvent::TextDelta { text: "Hello from the loop.".into() },
        ModelEvent::Stop { reason: StopReason::EndTurn },
    ]],
);
let harness = HarnessBuilder::new(Config::default())
    .host(Host::testing())
    .llm(Arc::new(scripted))
    .build()
    .await
    .expect("builds");

let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("say hello")]);
message.task_id = Some("t1".into());
let request = AgentRequest {
    message,
    call: CallContext {
        task_id: "t1".into(),
        context_id: "ctx".into(),
        stored_task: None,
        caller: Caller::human("me", [Role::Admin]),
        service_params: Default::default(),
    },
    metadata: None,
    extensions: vec![],
};
let events: Vec<_> = harness.agent().send_message(request).collect().await;
assert!(events.iter().any(|e| matches!(e, Ok(a2a::StreamResponse::Message(_)))));
# }
```

What the builder does, in order:

1. Takes the `Host` you give it (`Host::native()` on a machine, `Host::testing()` in a
   test, your own in the browser) and the `Config`.
2. Registers the models from `[models]`, or the ones you hand it with `.llm(..)`, and
   routes the loop to `default`.
3. Discovers and installs the plugins under `[plugins] paths`, connects their MCP
   servers, and registers the built-in tools and commands.
4. Opens the store (`memory` or `file`) and builds the `LocalAgent` with the in-process
   durable mode, so open tasks resume after a restart.

Everything is an entity in the `Registry` you can read back with `harness.registry()`;
every operation runs through the `HookDispatcher` at `harness.hooks()`, where you
register your own hooks before the first task.

`harness.send_message(request, None)` does the same from a plain
`a2a::SendMessageRequest`, acting as the operator; `get_task`, `cancel_task` and
`subscribe` complete the SDK surface the TypeScript and Python packages wrap.

## In the browser (TypeScript)

The npm package `rusty-harness` is the same harness compiled to WebAssembly. The page
hands in the platform as callbacks; only `secret` has no default.

```ts
import { Harness, type Config } from "rusty-harness";

const config: Config = {
  models: { default: { provider: "openai", model: "gpt-6.1", api_key: "env:OPENAI_API_KEY" } },
};
const harness = await Harness.create(config, {
  allowOrigins: ["https://api.openai.com"],
  secret: async (reference) => fetchKeyFromMyBackend(reference),
});
for await (const event of harness.sendMessage({
  message: { messageId: crypto.randomUUID(), role: "ROLE_USER", parts: [{ text: "hello" }] },
})) {
  console.log(event);
}
```

A key the page resolves lives in the page; for a public site, run the harness as a
server instead and point the web renderer at it. The
[browser capability](../capabilities/browser.md) lists what is unavailable there.

## Configuration

`Config` is one typed struct, loaded by `Config::load` with the precedence code →
environment (`RH__SECTION__KEY`) → file → defaults, and published as
`rusty-harness.schema.json`. The [configuration capability](../capabilities/config.md)
has the full list; `config.default.toml` in the crate is the commented default.

## Where to go next

- [The core loop](../capabilities/loop.md): what a task goes through.
- [Tools and MCP](../capabilities/tools.md): registering a tool of your own.
- [Write a plugin](./write-a-plugin.md): the same without code.

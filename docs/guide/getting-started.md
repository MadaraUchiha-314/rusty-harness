# Getting started

rusty-harness ships the same Rust core to three ecosystems. Pick yours. To run the agent
as a server instead of embedding it, see [Run the server](./run-the-server).

## The `rusty-harness` command

The binary (the TUI, the A2A server and first-run setup) installs from any of the three
registries, prebuilt for Linux (x64, arm64; glibc), macOS (x64, arm64) and Windows (x64),
with nothing downloaded at install time:

```sh
npm i -g @rusty-harness/cli          # one optional dependency per platform
pip install "rusty_harness[cli]"     # the rusty-harness-cli wheel, on PATH
cargo install rusty_harness --features cli
```

Each release also attaches the archives (`rusty-harness-<version>-<platform>.tar.gz`,
`.zip` on Windows) and their `SHA256SUMS` to the GitHub release. See
[Run the TUI](./run-the-tui) for the first run.

## Rust

```sh
cargo add rusty_harness --features rt-tokio
cargo add tokio --features macros,rt-multi-thread
cargo add a2a-lf futures   # the official A2A types (`use a2a::…`) and stream helpers
```

```rust
use futures::StreamExt as _;
use rusty_harness::{HarnessBuilder, config::Config};

#[tokio::main]
async fn main() {
    let config = Config::from_layers(Some(r#"
        [models.default]
        provider = "openai"
        model = "gpt-6.1-sol"
        api_key = "env:OPENAI_API_KEY"
    "#), std::env::vars(), None).expect("config");
    let harness = HarnessBuilder::new(config).build().await.expect("harness");
    let message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("Hello!")]);
    let request = a2a::SendMessageRequest { message, configuration: None, metadata: None, tenant: None };
    let mut events = harness.send_message(request, None);
    while let Some(event) = events.next().await {
        println!("{event:?}");
    }
}
```

The core crate has no default runtime. Enable `rt-tokio` for the tokio-backed
`Runtime`, or none at all if you only call APIs that need no runtime — see the
[async model](./async-model). [Embed the SDK](./embed-the-sdk) covers the builder,
`get_task`, `cancel_task` and `subscribe`.

## Python

```sh
pip install rusty_harness
```

```python
import asyncio
from rusty_harness import Config, Harness

config = Config.from_dict({
    "models": {"default": {"provider": "openai", "model": "gpt-6.1-sol", "api_key": "env:OPENAI_API_KEY"}},
})
request = {"message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": "Hello!"}]}}

async def main() -> None:
    harness = await Harness.create(config)
    async for event in harness.send_message(request):
        print(event)

asyncio.run(main())

# Blocking variant, for scripts and the REPL:
for event in Harness(config).send_message_sync(request):
    print(event)
```

Wheels are abi3 (CPython 3.10+), one per platform, and ship type stubs.

## JavaScript and TypeScript

```sh
npm i rusty-harness
```

```ts
import { Harness } from "rusty-harness";

const harness = await Harness.create(
  { models: { default: { provider: "openai", model: "gpt-6.1-sol", api_key: "env:OPENAI_API_KEY" } } },
  { secret: async (reference) => resolveOnYourBackend(reference) }, // no default in the browser
);
for await (const event of harness.sendMessage({
  message: { messageId: crypto.randomUUID(), role: "ROLE_USER", parts: [{ text: "Hello!" }] },
})) {
  console.log(event);
}
```

The default entry point serves Node (`exports.node`) and bundlers such as Vite or webpack.
For a plain `<script type="module">` page with no bundler, use the `web` build and
initialise the module first:

```ts
import init, { Harness } from "rusty-harness/web";

await init();
```

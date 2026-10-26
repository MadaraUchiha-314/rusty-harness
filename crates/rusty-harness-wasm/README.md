# rusty-harness (npm)

A tiny agent harness that's blazing fast — the Rust core compiled to WebAssembly.

```sh
npm i rusty-harness
```

```ts
import { Harness, type Config } from "rusty-harness";

const config: Config = {
  models: { default: { provider: "openai", model: "gpt-6.1", api_key: "env:OPENAI_API_KEY" } },
};
const harness = await Harness.create(config, {
  allowOrigins: ["https://api.openai.com"],
  secret: async (reference) => resolveOnMyBackend(reference), // no default
});

for await (const event of harness.sendMessage({
  message: { messageId: crypto.randomUUID(), role: "ROLE_USER", parts: [{ text: "hello" }] },
})) {
  console.log(event); // { task } | { message } | { statusUpdate } | { artifactUpdate }
}
```

`Harness.create` also takes the TOML configuration as text. `cancelTask`, `getTask` and
`subscribe` complete the surface; a rejected call is an `Error` with the A2A `code`.

The host callbacks (`fetch`, `webSocket`, `now`, `randomBytes`, `log`, `secret`) default
to the platform's own except `secret`. Without a bundler, in the browser, use the `web`
entry point and call `init()` first:

```ts
import init, { Harness } from "rusty-harness/web";

await init();
```

Unavailable in the browser: file plugins and the `file` store, stdio MCP servers,
`embedded`/`remote` durability, the TUI and the A2A server. See the
[documentation site](https://madarauchiha-314.github.io/rusty-harness/capabilities/browser).

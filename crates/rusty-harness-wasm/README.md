# rusty-harness (npm)

A tiny agent harness that's blazing fast — the Rust core compiled to WebAssembly.

```sh
npm i rusty-harness
```

```ts
import { hello } from "rusty-harness";

console.log(await hello()); // Promise<string>
```

Without a bundler, in the browser, use the `web` entry point and call `init()` first:

```ts
import init, { hello } from "rusty-harness/web";

await init();
```

See the [documentation site](https://madarauchiha-314.github.io/rusty-harness/).

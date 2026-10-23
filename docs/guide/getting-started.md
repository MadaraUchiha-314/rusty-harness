# Getting started

rusty-harness ships the same Rust core to three ecosystems. Pick yours.

## Rust

```sh
cargo add rusty_harness --features rt-tokio
cargo add tokio --features macros,rt-multi-thread
```

```rust
#[tokio::main]
async fn main() {
    println!("{}", rusty_harness::hello().await);
}
```

The core crate has no default runtime. Enable `rt-tokio` for the tokio-backed
`Runtime`, or none at all if you only call APIs that need no runtime — see the
[async model](./async-model).

## Python

```sh
pip install rusty_harness
```

```python
import asyncio
import rusty_harness

async def main() -> None:
    print(await rusty_harness.hello())

asyncio.run(main())

# Blocking variant, for scripts and the REPL:
print(rusty_harness.hello_sync())
```

Wheels are abi3 (CPython 3.10+), one per platform, and ship type stubs.

## JavaScript and TypeScript

```sh
npm i rusty-harness
```

```ts
import { hello } from "rusty-harness";

console.log(await hello()); // hello(): Promise<string>
```

The default entry point serves Node (`exports.node`) and bundlers such as Vite or webpack.
For a plain `<script type="module">` page with no bundler, use the `web` build and
initialise the module first:

```ts
import init, { hello } from "rusty-harness/web";

await init();
console.log(await hello());
```

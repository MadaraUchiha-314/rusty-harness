# Async model

The harness is async at every layer, and the core crate is **runtime-agnostic**.

```text
rusty_harness  (async fn APIs, Runtime trait — no runtime by default)
├── rt-tokio → TokioRuntime ─┬─ Rust applications
│                            └─ rusty-harness-py   (awaitables + *_sync)
└── rt-wasm  → WasmRuntime  ─── rusty-harness-wasm (Promise exports)
```

## Rules the core follows

- Public APIs are `async fn` or return a plain `impl Future`. No runtime type appears in a
  public signature.
- Anything that needs a runtime — spawning, timers, I/O — goes through the `Runtime`
  trait in `rusty_harness::runtime`, with two feature-gated implementations:
  - `rt-tokio` — `TokioRuntime`, for native consumers and the Python bindings.
  - `rt-wasm` — `WasmRuntime` (`wasm-bindgen-futures::spawn_local`, `gloo-timers`),
    compiled only for `wasm32`.
- There is **no default runtime feature**. `tokio` is absent from the core's dependency
  tree unless `rt-tokio` is on, and CI compiles the core for `wasm32-unknown-unknown` on
  every pull request to keep it that way.
- Bounds use `MaybeSend`, which is `Send` on native targets and empty on `wasm32`, where
  JS futures are `!Send` and there is one thread.

## Bring your own runtime

APIs that need no runtime work under any executor.

With tokio:

```rust
#[tokio::main]
async fn main() {
    println!("{}", rusty_harness::hello().await);
}
```

With smol:

```rust
fn main() {
    smol::block_on(async {
        println!("{}", rusty_harness::hello().await);
    });
}
```

APIs that do need a runtime take one explicitly:

```rust
use rusty_harness::{Runtime, runtime::TokioRuntime};

#[tokio::main]
async fn main() {
    let rt = TokioRuntime::current();
    rt.spawn(async { println!("{}", rusty_harness::hello().await) });
    rt.sleep(std::time::Duration::from_millis(10)).await;
}
```

## In the bindings

- **Python** — every async API is an awaitable driven by one shared, multi-thread tokio
  runtime owned by the extension module (`pyo3-async-runtimes`); nothing creates a runtime
  per call. Each also has a blocking `*_sync` twin that releases the GIL while it waits.
- **JavaScript** — every async API returns a `Promise`, bridged by `wasm-bindgen-futures`.

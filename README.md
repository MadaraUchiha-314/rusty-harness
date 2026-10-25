# rusty-harness

A tiny agent harness that's blazing fast. One Rust core, shipped to three ecosystems:

| Ecosystem | Install | Package |
|---|---|---|
| Rust | `cargo add rusty_harness --features rt-tokio` | [crates.io](https://crates.io/crates/rusty_harness) |
| Python | `pip install rusty_harness` | [PyPI](https://pypi.org/project/rusty_harness/) |
| JS / TS | `npm i rusty-harness` | [npm](https://www.npmjs.com/package/rusty-harness) |
| The `rusty-harness` command | `npm i -g @rusty-harness/cli` · `pip install "rusty_harness[cli]"` | [npm](https://www.npmjs.com/package/@rusty-harness/cli) · [PyPI](https://pypi.org/project/rusty-harness-cli/) |

All logic lives in [`crates/rusty-harness`](crates/rusty-harness); the Python
([`crates/rusty-harness-py`](crates/rusty-harness-py)) and WebAssembly
([`crates/rusty-harness-wasm`](crates/rusty-harness-wasm)) crates are thin adapters.

Documentation, including local development for every layer, is at
<https://madarauchiha-314.github.io/rusty-harness/> and in [`docs/`](docs).

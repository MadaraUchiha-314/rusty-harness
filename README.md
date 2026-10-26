# rusty-harness

A tiny agent harness that's blazing fast. One Rust core runs the agent loop, serves it
over [A2A 1.0](https://a2a-protocol.org/), and ships to Rust, Python and JavaScript:

- **One agent, every surface.** A terminal UI and a web renderer, both plain A2A clients
  of the same harness, with A2UI forms, images, voice and slash commands.
- **Long-running tasks.** A task stays open across turns, asks people for input through
  channels, plans its work as a DAG, and survives restarts on Temporal (or runs in
  process).
- **Extensible.** Tools, MCP servers, skills, plugins and hooks on every operation;
  models from OpenAI (Responses API) and Anthropic, or any OpenAI-compatible endpoint.
- **Small and fast.** About 260 µs of harness per turn, a 26 ms TUI cold start and a
  20 MB binary.

| Get | Install | Package |
|---|---|---|
| The `rusty-harness` command | `npm i -g @rusty-harness/cli` · `pip install "rusty_harness[cli]"` | [npm](https://www.npmjs.com/package/@rusty-harness/cli) · [PyPI](https://pypi.org/project/rusty-harness-cli/) |
| Rust | `cargo add rusty_harness --features rt-tokio` | [crates.io](https://crates.io/crates/rusty_harness) |
| Python | `pip install rusty_harness` | [PyPI](https://pypi.org/project/rusty_harness/) |
| JS / TS (WebAssembly) | `npm i rusty-harness` | [npm](https://www.npmjs.com/package/rusty-harness) |

## Quick start

```sh
export OPENAI_API_KEY=…   # referenced as env:OPENAI_API_KEY, never stored
rusty-harness             # writes ~/.rusty-harness/config.toml, starts the harness, opens the TUI
rusty-harness --no-tui    # the A2A server alone, on 127.0.0.1:7474
```

The [demo](docs/guide/run-the-demo.md) runs the whole thing (the TUI and the web
renderer on one harness, an A2UI ask, a task open across turns, Temporal Cloud) from a
checked-in configuration.

## Layout

All logic lives in [`crates/rusty-harness`](crates/rusty-harness); the Python
([`crates/rusty-harness-py`](crates/rusty-harness-py)) and WebAssembly
([`crates/rusty-harness-wasm`](crates/rusty-harness-wasm)) crates are thin adapters, and
[`web/`](web) is the web renderer.

Documentation, including the architecture, the capabilities and local development for
every layer, is at <https://madarauchiha-314.github.io/rusty-harness/> and in
[`docs/`](docs).

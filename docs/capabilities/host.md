# Capability: the host

> The only door through which the core touches the platform: every capability is a
> typed interface, native by default, supplied by the page on `wasm32`, and a missing one
> fails at construction, never at first use.

## What it is

`Host` holds one implementation per capability: `HttpClient`, `WebSocketClient`,
`Clock`, `Rng`, `SecretResolver`, `FileSystem`, `ProcessSpawner`, `McpTransport`,
`Microphone`, `Speaker` and `LogSink`. `Host::native()` supplies reqwest,
tokio-tungstenite (TLS through rustls with the OS roots), the system clock and RNG,
`env:`/`keyring:`/`file:` secrets, tokio's file system and processes, and cpal audio
under the `audio` feature. On `wasm32` the npm package builds the host from the page's
callbacks ([the browser](browser.md)). `Host::testing()` is a fully mocked host for
tests.

## Current behaviour

- WHEN an entity needs a capability the host does not supply THEN it SHALL fail at
  construction with `Unsupported(<capability>)` (R29.3).
- WHEN a credential reference is resolved natively THEN `env:NAME` SHALL read the
  environment, `keyring:SERVICE/USER` SHALL run `secret-tool lookup service … username …`,
  `keyring:ATTR=VALUE,…` SHALL look the entry up by those attributes in order, and
  `file:PATH` SHALL read the file (`~/` under `$HOME`), trimmed; the value is wrapped as
  a `Secret` that is never logged, serialized or echoed in an error.
- WHEN a child process is spawned (a stdio MCP server, the embedded Temporal server)
  THEN it SHALL see only the environment the caller lists, plus `PATH`.
- WHEN audio is captured or played THEN it SHALL be 16-bit PCM at 24 kHz mono at the
  harness's boundary, resampled from the device's format.
- WHEN a credential or store path is under `~` and HOME is not set THEN it SHALL be
  refused, not read relative to the working directory.

## Design

- [`design.md` § Host capabilities][design] (R4.7, R29).
- Code: `crates/rusty-harness/src/host/` (native, audio, secret, testing);
  `crates/rusty-harness-wasm/src/host.rs` (the JS host).

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M7, task 46) | `keyring:ATTR=VALUE,…` and `file:~/…` references | PR #13 |
| issue-3 (M7, task 42) | The host built from page callbacks on `wasm32` | PR #13 |
| issue-3 (M6, tasks 39–40) | Native WebSocket, cpal microphone and speaker | PR #12 |
| issue-3 (M1, task 4) | `Host`, the capability traits, the native and testing hosts, secrets | PR #7 |
| issue-3 (M7, SR1-15, SR1-16) | `keyring:` takes the attribute form whenever an `=` precedes the first `/`; a path under `~` is refused when HOME is not set | PR #13 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

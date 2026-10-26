# Run the demo

The definition-of-done demo (R32): one harness that drafts release notes with you, reached
from the TUI and the web renderer at once. It asks who the notes are for through an A2UI
form, waits for your answer, drafts, and keeps the task open for revisions. It runs on
OpenAI's GPT 6.1 and either in process (`none`) or on Temporal Cloud (`remote`).

## What it needs

- `OPENAI_API_KEY` in the environment (the config names `env:OPENAI_API_KEY`).
- For `remote`, a Temporal Cloud namespace. The config reads every value by reference:
  the address and namespace from `~/.config/rusty-harness/temporal-address` and
  `temporal-namespace`, the API key from the keyring entry
  `service temporal project rusty-harness`. Edit
  `crates/rusty-harness/tests/fixtures/demo/config.remote.toml` to point elsewhere, or set
  `from_env = true` to use `TEMPORAL_ADDRESS`, `TEMPORAL_NAMESPACE` and `TEMPORAL_API_KEY`.

## Run it

From `crates/rusty-harness/`:

```sh
cargo run --features cli -- --config tests/fixtures/demo/config.toml --no-tui         # none
cargo run --features cli -- --config tests/fixtures/demo/config.remote.toml --no-tui  # remote
```

Then attach both renderers to `http://127.0.0.1:7474/`:

```sh
cargo run --features cli -- --url http://127.0.0.1:7474/   # the TUI
bun run --cwd web dev                                     # the web renderer; open it with ?url=http://127.0.0.1:7474/
```

Ask for release notes, answer the form, then ask for a revision. The task stays
`input-required`, waiting for your next turn, until you complete it.

## As tests

The same journey is an end-to-end test per mode, ignored by default because it spends
tokens. The browser leg runs against a demo you started.

```sh
cargo nextest run -p rusty_harness --features cli -E 'test(e2e_none_)' --run-ignored all
cargo nextest run -p rusty_harness --features cli -E 'test(e2e_remote_)' --run-ignored all
RH_DEMO_URL=http://127.0.0.1:7474/ PW_VIDEO=1 bunx --cwd web playwright test --project=e2e demo.spec.ts
```

`RH_E2E_EVIDENCE=<dir>` keeps each run's TUI frames, task, trace and stills.
`scripts/demo-history.sh <task id> <out.json>` fetches a `remote` task's workflow history
from Temporal Cloud with payloads, address and namespace scrubbed. To record both
renderers side by side, serve the TUI in a browser terminal and run the recording spec:

```sh
ttyd -W -p 7681 -t rendererType=dom target/debug/rusty-harness --url http://127.0.0.1:7474/ &
RH_DEMO_URL=http://127.0.0.1:7474/ RH_DEMO_TTYD_URL=http://127.0.0.1:7681/ PW_VIDEO=1 \
  bunx --cwd web playwright test --project=e2e demo-recording.spec.ts
```

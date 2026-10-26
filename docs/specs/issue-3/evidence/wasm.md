# T12 — WASM and the browser

<!-- markdownlint-disable MD013 -->

The core on `wasm32`: it compiles without tokio, runs a task against a mocked host in Node and headless Chrome, and the npm package builds and passes its tests.

## wasm32 check, and tokio absent from the tree

```sh
cargo check --target wasm32-unknown-unknown -p rusty_harness --no-default-features --features rt-wasm && ! cargo tree -p rusty_harness --no-default-features --features rt-wasm --target wasm32-unknown-unknown -e normal | grep -E '^.*tokio v'
```

Outcome: **passed** in 9s, 2026-10-25, at `0a97f5b`. The second command is `! cargo tree … | grep tokio`: it passes only when no tokio line is found.

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.98s
```

## wasm-pack test --node

```sh
wasm-pack test --node
```

Outcome: **passed** in 47s, 2026-10-25, at `0a97f5b`.

```text
Running unittests src/lib.rs (<repo>/target/wasm32-unknown-unknown/debug/deps/rusty_harness_wasm-a5a06964162b9159.wasm)
     Running tests/browser.rs (<repo>/target/wasm32-unknown-unknown/debug/deps/browser-8ecd6efa4df0a724.wasm)
     Running tests/node.rs (<repo>/target/wasm32-unknown-unknown/debug/deps/node-47b3cbb0c02fb10b.wasm)
running 4 tests
test the_wasm_build_runs_a_task_against_a_mocked_host ... ok
test hello_resolves ... ok
test an_unavailable_capability_fails_at_construction_on_wasm32 ... ok
test a_fetch_outside_allow_origins_is_refused ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 filtered out; finished in 0.25s
```

## wasm-pack test --headless --chrome

```sh
wasm-pack test --headless --chrome --chromedriver /usr/bin/chromedriver
```

Outcome: **passed** in 16s, 2026-10-25, at `0a97f5b`.

```text
Running unittests src/lib.rs (<repo>/target/wasm32-unknown-unknown/debug/deps/rusty_harness_wasm-a5a06964162b9159.wasm)
     Running tests/browser.rs (<repo>/target/wasm32-unknown-unknown/debug/deps/browser-8ecd6efa4df0a724.wasm)
Running headless tests in Chrome on `http://127.0.0.1:39565/`
running 3 tests
test the_wasm_build_runs_a_task_against_a_mocked_host ... ok
test hello_resolves_in_the_browser ... ok
test a_fetch_outside_allow_origins_is_refused ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 filtered out; finished in 0.20s
     Running tests/node.rs (<repo>/target/wasm32-unknown-unknown/debug/deps/node-47b3cbb0c02fb10b.wasm)
This test suite is only configured to run in node.js, but we're only running
```

## npm package build and tests

```sh
bun run build && bun run test
```

Outcome: **passed** in 230s, 2026-10-25, at `0a97f5b`.

```text
[INFO]: ✨   Done in 1m 54s
[INFO]: 📦   Your wasm pkg is ready to publish at <repo>/crates/rusty-harness-wasm/dist/bundler.
[INFO]: ✨   Done in 56.83s
[INFO]: 📦   Your wasm pkg is ready to publish at <repo>/crates/rusty-harness-wasm/dist/web.
[INFO]: ✨   Done in 57.67s
[INFO]: 📦   Your wasm pkg is ready to publish at <repo>/crates/rusty-harness-wasm/dist/nodejs.
 Test Files  2 passed (2)
      Tests  7 passed (7)
```

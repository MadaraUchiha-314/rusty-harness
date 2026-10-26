# T16 — static checks

<!-- markdownlint-disable MD013 -->

The repository's lint floor, as CI runs it.

## rustfmt

```sh
cargo fmt --all --check
```

Outcome: **passed** in 1s, 2026-10-25, at `0a97f5b`.

```text
(no output)
```

## Clippy, every target and feature, warnings denied

```sh
cargo clippy --all-targets --all-features -- -D warnings
```

Outcome: **passed** in 5s, 2026-10-25, at `0a97f5b`.

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.80s
```

## rustdoc, warnings denied

```sh
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features --workspace
```

Outcome: **passed** in 26s, 2026-10-25, at `0a97f5b`.

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 25.42s
   Generated <repo>/target/doc/rusty_harness_py/index.html and 2 other files
```

## cargo deny

```sh
cargo deny check
```

Outcome: **passed** in 1s, 2026-10-25, at `0a97f5b`. The duplicate-version warnings are allowed by `deny.toml` (`multiple-versions = "warn"`); every check reports ok.

```text
warning[no-license-field]: license expression was not specified in manifest for crate 'temporalio-client = 1.1.0'
warning[no-license-field]: license expression was not specified in manifest for crate 'temporalio-common = 1.1.0'
warning[no-license-field]: license expression was not specified in manifest for crate 'temporalio-common-wasm = 1.1.0'
warning[no-license-field]: license expression was not specified in manifest for crate 'temporalio-macros = 1.1.0'
warning[no-license-field]: license expression was not specified in manifest for crate 'temporalio-protos = 0.10.0'
warning[no-license-field]: license expression was not specified in manifest for crate 'temporalio-sdk = 1.1.0'
warning[no-license-field]: license expression was not specified in manifest for crate 'temporalio-sdk-core = 0.10.0'
warning[no-license-field]: license expression was not specified in manifest for crate 'temporalio-workflow = 1.1.0'
warning[duplicate]: found 2 duplicate entries for crate 'base64'
warning[duplicate]: found 2 duplicate entries for crate 'core-foundation'
warning[duplicate]: found 2 duplicate entries for crate 'foldhash'
warning[duplicate]: found 3 duplicate entries for crate 'getrandom'
warning[duplicate]: found 2 duplicate entries for crate 'hashbrown'
warning[duplicate]: found 3 duplicate entries for crate 'itertools'
warning[duplicate]: found 2 duplicate entries for crate 'jni-sys'
warning[duplicate]: found 2 duplicate entries for crate 'linux-raw-sys'
warning[duplicate]: found 2 duplicate entries for crate 'logos'
warning[duplicate]: found 2 duplicate entries for crate 'logos-codegen'
warning[duplicate]: found 2 duplicate entries for crate 'logos-derive'
warning[duplicate]: found 2 duplicate entries for crate 'miniz_oxide'
warning[duplicate]: found 2 duplicate entries for crate 'prettyplease'
warning[duplicate]: found 2 duplicate entries for crate 'r-efi'
warning[duplicate]: found 3 duplicate entries for crate 'rand'
warning[duplicate]: found 2 duplicate entries for crate 'rand_chacha'
warning[duplicate]: found 3 duplicate entries for crate 'rand_core'
warning[duplicate]: found 2 duplicate entries for crate 'rustc-hash'
warning[duplicate]: found 2 duplicate entries for crate 'rustix'
warning[duplicate]: found 2 duplicate entries for crate 'syn'
warning[duplicate]: found 2 duplicate entries for crate 'thiserror'
warning[duplicate]: found 2 duplicate entries for crate 'thiserror-impl'
warning[duplicate]: found 2 duplicate entries for crate 'tower-http'
warning[duplicate]: found 2 duplicate entries for crate 'unicode-width'
warning[duplicate]: found 2 duplicate entries for crate 'windows'
warning[duplicate]: found 2 duplicate entries for crate 'windows-core'
warning[duplicate]: found 2 duplicate entries for crate 'windows-implement'
warning[duplicate]: found 2 duplicate entries for crate 'windows-interface'
warning[duplicate]: found 2 duplicate entries for crate 'windows-result'
warning[duplicate]: found 2 duplicate entries for crate 'windows-strings'
warning[duplicate]: found 2 duplicate entries for crate 'windows-sys'
warning[duplicate]: found 2 duplicate entries for crate 'wit-bindgen'
advisories ok, bans ok, licenses ok, sources ok
```

## cargo machete

```sh
cargo machete
```

Outcome: **passed** in 0s, 2026-10-25, at `0a97f5b`.

```text
Analyzing dependencies of crates in this directory...
cargo-machete didn't find any unused dependencies in this directory. Good job!
Done!
```

## oxlint, oxfmt, markdownlint, tsc (web)

```sh
bun run lint && bun run format && bun run lint:md && bunx tsc --noEmit -p web
```

Outcome: **passed** in 9s, 2026-10-25, at `0a97f5b`.

```text
$ oxlint --type-aware
$ oxfmt --check

All matched files use the correct format.
Finished in 147ms on 154 files using 4 threads.
$ markdownlint-cli2
markdownlint-cli2 v0.24.0 (markdownlint v0.42.0)
Finding: **/*.md !**/node_modules/** !**/dist/** !target/** !CHANGELOG.md !crates/rusty-harness-py/.venv/** !web/test-results/** !web/playwright-report/**
Linting: 107 files
Summary: 0 issues in 0 files
```

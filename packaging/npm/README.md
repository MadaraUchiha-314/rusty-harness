# @rusty-harness/cli

The `rusty-harness` binary on npm: the TUI, the A2A server and first-run setup. npm
installs the one prebuilt binary that matches your machine (an optional dependency per
platform); nothing is downloaded at install time.

```sh
npm i -g @rusty-harness/cli
rusty-harness            # first run writes ~/.rusty-harness/config.toml and opens the TUI
rusty-harness serve      # the A2A server
```

Prebuilt for `linux-x64`, `linux-arm64` (glibc), `darwin-x64`, `darwin-arm64` and
`win32-x64`. Elsewhere: `cargo install rusty_harness --features cli`.

The SDK is the separate `rusty-harness` package. See the
[documentation site](https://madarauchiha-314.github.io/rusty-harness/).

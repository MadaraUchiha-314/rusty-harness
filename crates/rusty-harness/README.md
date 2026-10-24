# rusty_harness

A tiny agent harness that's blazing fast. This is the core crate: every piece of logic
lives here, and the Python (`rusty_harness` on PyPI) and JavaScript (`rusty-harness` on
npm) packages are thin bindings over it.

```sh
cargo add rusty_harness --features rt-tokio
```

See the [documentation site](https://madarauchiha-314.github.io/rusty-harness/).

# Decision 006: the shipped binary is built size-first, and aborts on panic

- **Status:** accepted
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3)
- **Design:** [`docs/specs/issue-3/design.md`][design] § Packaging and SDK parity;
  open question Q8; testing plan T7 (binary < 25 MB)

## Decision

The `rusty-harness` binary that npm, PyPI and the GitHub release ship is built with a
dedicated Cargo profile, `dist`, inheriting `release`:

```toml
[profile.dist]
inherits = "release"
opt-level = "s"
lto = "fat"
codegen-units = 1
strip = true
panic = "abort"
```

`release` itself is unchanged, so benchmarks (T7's `loop_overhead`) and crates that depend
on `rusty_harness` keep the default speed-first settings and unwinding.

## Why

Measured on linux-x64 with `--features cli`:

| Profile | Bytes |
|---|---|
| `release` (default) | 67 579 057 |
| + strip, fat LTO, one codegen unit | 36 454 920 |
| + `opt-level = "s"` | 23 663 432 |
| + `panic = "abort"` (the `dist` profile) | 19 733 816 |

The budget is 25 MB. `opt-level = "s"` alone fits, but with 1.3 MB of headroom on one
platform, so the next dependency, or a platform whose binaries run larger, would break it.

Aborting on panic is also the behaviour this binary should have. The core catches no
panics (no `catch_unwind`, no inspection of `JoinError`). A panic in a spawned task
under unwinding ends that task silently and can leave its A2A task in `working`. An
abort ends the process, and the durable modes resume open tasks on restart (R14.4), so
the failure is visible and recovered from.

## Consequences

- A panic anywhere in the shipped binary ends the process, and the panic message is the
  last thing on stderr.
- Every channel builds with `--profile dist`: `binaries.yml`, the CLI wheel (maturin
  `profile = "dist"`) and `scripts/measure.sh`. `binaries.yml` reports each platform's
  size and warns over 25 MB; the targets are measured, not gates (Q13).
- Fat LTO makes the shipped build slow (about ten minutes locally), which is why it is a
  separate profile and not part of `cargo build --release`.

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

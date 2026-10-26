# Learning 003 — Pin the widget crate that matches the ratatui line, even when it is a release candidate

- **Date:** 2026-10-25
- **Source:** system-feedback (dependency resolution while adding the TUI, issue #3 task 34)
- **Category:** tooling

## What happened

The design names ratatui 0.30 with `tui-markdown` and `ratatui-image`. `ratatui-image`'s
stable line (11.x) targets ratatui 0.29; only the 12.0.0 release candidates target 0.30, so
the stable crate would have pulled a second ratatui into the tree. Its `image-defaults`
feature also enables every `image` decoder, and the AVIF encoder's `rav1e` brings
`libfuzzer-sys` under the NCSA licence, which `cargo deny` rejects.

## Learning

Pick the widget crate version by the ratatui line it depends on (check the sparse index
entry's `deps`), accept a release candidate when it is the only match and say so in
`Cargo.toml`, and turn off decoder bundles you do not need: `image` with `png`, `jpeg`,
`gif` and `webp` only keeps the licence set as it was.

## Applied in

`Cargo.toml` (workspace `ratatui-image = 12.0.0-rc.3`, `image` with four decoders),
[capability: the terminal renderer](../capabilities/tui.md).

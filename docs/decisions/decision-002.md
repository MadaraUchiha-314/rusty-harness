# Decision 002: MSRV 1.92 for the Temporal Rust SDK

- **Status:** accepted
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3)
- **Design:** [`docs/specs/issue-3/design.md`](../specs/issue-3/design.md) § Trade-offs (T7),
  open question D3

## Decision

The workspace's minimum supported Rust version rises from 1.88 to **1.92**. The official
Temporal Rust SDK (`temporalio-sdk` 1.1, the `durable-temporal` feature) declares
`rust-version = "1.92"`, and decision-001 keeps one MSRV for the whole workspace rather
than a per-crate value. Releases 1.88 to 1.91 are no longer supported; the CI `msrv` leg
builds on 1.92.

## Rejected

Isolating Temporal in a crate with its own MSRV would keep 1.88 for the core at the cost
of a second MSRV to document, test and explain, for a feature most users of the binary
turn on.

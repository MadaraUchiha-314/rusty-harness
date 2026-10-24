# Decision 003: the official A2A Rust SDK, protocol 1.0 or later, own browser transport

- **Status:** accepted
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3)
- **Design:** [`docs/specs/issue-3/design.md`][design] § Trade-offs (T1, T2);
  requirements S9, R19.5, R19.9, Q2

## Decision

The harness speaks A2A through the official Rust SDK (`a2a-lf` for the types, `a2a-server-lf`
and `a2a-client-lf` for the server and native client, `a2a-grpc` behind a feature), pinned,
and never hand-rolls protocol types. **Protocol version 1.0 or later is a hard
constraint** on every dependency, surface and peer (S9); the crates' 0.x version numbers
describe their API stability, not the protocol they implement.

Two consequences are accepted:

- Where the SDK lacks a 1.0 feature (today: `A2A-Extensions` negotiation and the caller's
  identity on the executor context), the gap is closed in one marked module,
  `app_service::a2a_server::protocol_gap`, each item tracked upstream and removed when the
  SDK ships it. The gap is never closed by speaking an older protocol.
- On `wasm32` the SDK's client does not build (tokio), so the browser uses the official
  *types* with the crate's own JSON-RPC transport over the host's `fetch`
  (`harness::a2a_client::wasm_transport`). The types crate builds there once the workspace
  enables `uuid`'s `js` feature on `wasm32` (feature unification), which the CI `wasm-check`
  job proves on every pull request.

## Rejected

The same-named community crate `a2a-rs` (not official); waiting for upstream browser
support before shipping the WASM build; forking the client crate.

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

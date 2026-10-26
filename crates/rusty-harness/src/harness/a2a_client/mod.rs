//! Outbound A2A: `RemoteAgent`, `LoopbackAgent`, and the browser transport over the host's `fetch` (R17, R15.3).

#[cfg(target_arch = "wasm32")]
pub mod wasm_transport;

//! Tools: the `Tool` trait, the executor, MCP servers, the memory and sandbox seams (R8).

#[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
pub mod mcp;
pub mod memory;
pub mod sandbox;

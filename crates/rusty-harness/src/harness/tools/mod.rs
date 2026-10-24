//! Tools: the `Tool` trait, the executor, MCP servers, the memory and sandbox seams (R8).
//!
//! Task 11 defines what a model sees of a tool ([`ToolDescriptor`], [`ToolCall`]); task
//! 15 adds the `Tool` trait and the executor.

use serde::{Deserialize, Serialize};

#[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
pub mod mcp;
pub mod memory;
pub mod sandbox;

/// Whether a tool may be retried after a failure without harm (R22.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Idempotency {
    /// Running it twice has the same effect as once.
    Idempotent,
    /// It has side effects; `accepts_key` says whether an idempotency key makes a retry safe.
    SideEffecting {
        /// The tool deduplicates on the key the harness passes.
        accepts_key: bool,
    },
}

/// What the model is told about a tool, and what the executor validates a call against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// The name the model calls; unique in a request.
    pub name: String,
    /// What the tool does, for the model.
    pub description: String,
    /// The JSON Schema of the arguments.
    pub parameters: serde_json::Value,
    /// Retry safety.
    pub idempotency: Idempotency,
}

/// One tool call the model asked for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    /// The provider's id for the call, echoed back with the result.
    pub id: String,
    /// The tool's name.
    pub name: String,
    /// The arguments, as the model produced them (validated by the executor).
    pub arguments: serde_json::Value,
}

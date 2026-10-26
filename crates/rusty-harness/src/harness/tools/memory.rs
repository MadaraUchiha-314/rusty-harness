//! The memory-provider seam: a typed extension point, nothing implemented in issue-3 (S2).

use crate::harness_async_trait;
use crate::registry::Entity;

/// What a memory provider is asked to recall.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryQuery {
    /// The task asking.
    pub task: String,
    /// Free text.
    pub query: String,
    /// At most this many items.
    pub limit: usize,
}

/// One recalled item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryItem {
    /// Where it came from.
    pub source: String,
    /// The text.
    pub text: String,
}

harness_async_trait! {
    /// A memory provider (S2): the seam a plugin fills; the harness ships none.
    pub trait MemoryProvider: Entity {
        /// Recalls items for `query`.
        async fn recall(&self, query: MemoryQuery) -> Result<Vec<MemoryItem>, super::ToolError>;
        /// Remembers `text` for `task`.
        async fn remember(&self, task: &str, text: String) -> Result<(), super::ToolError>;
    }
}

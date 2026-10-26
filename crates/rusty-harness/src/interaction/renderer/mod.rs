//! The `Renderer` trait, the A2UI basic-catalog model and the terminal renderer (R25.4–6, R27).
//!
//! A native renderer is an entity that owns a [`SurfaceCapabilities`] and renders
//! [`RenderItem`]s: the A2A objects a task streams, an A2UI payload, and — behind the
//! `mcp_apps` capability flag — an MCP App nobody produces yet (R25.5). The web
//! renderer is TypeScript and talks A2A directly; the TUI implements the trait.

pub mod a2ui;
#[cfg(feature = "tui")]
pub mod tui;

use serde_json::Value;

use crate::harness_async_trait;
use crate::interaction::surface::SurfaceCapabilities;
use crate::registry::Entity;

/// What a renderer is handed, one at a time, in stream order.
#[derive(Clone, Debug, PartialEq)]
pub enum RenderItem {
    /// A message.
    Message(a2a::Message),
    /// A task snapshot (the first event of a stream).
    Task(a2a::Task),
    /// A status update; its message may carry a question.
    StatusUpdate(a2a::TaskStatusUpdateEvent),
    /// An artifact chunk.
    ArtifactUpdate(a2a::TaskArtifactUpdateEvent),
    /// An A2UI payload lifted out of a part.
    A2ui(Vec<a2ui::A2uiMessage>),
    /// A tool UI through MCP Apps (R25.5): left open, produced by nothing in this work item.
    McpApp(McpApp),
}

impl RenderItem {
    /// The item's name, for errors.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Message(_) => "message",
            Self::Task(_) => "task",
            Self::StatusUpdate(_) => "status-update",
            Self::ArtifactUpdate(_) => "artifact-update",
            Self::A2ui(_) => "a2ui",
            Self::McpApp(_) => "mcp-app",
        }
    }
}

impl From<a2a::StreamResponse> for RenderItem {
    fn from(response: a2a::StreamResponse) -> Self {
        match response {
            a2a::StreamResponse::Message(m) => Self::Message(m),
            a2a::StreamResponse::Task(t) => Self::Task(t),
            a2a::StreamResponse::StatusUpdate(u) => Self::StatusUpdate(u),
            a2a::StreamResponse::ArtifactUpdate(u) => Self::ArtifactUpdate(u),
        }
    }
}

/// A tool UI per MCP Apps (<https://modelcontextprotocol.io/extensions/apps/overview>):
/// the tool's result and the `ui://` resource that renders it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpApp {
    /// The tool whose result it shows.
    pub tool: String,
    /// The `ui://` resource of the app.
    pub resource_uri: String,
    /// The tool's result, as the app receives it.
    pub result: Value,
}

/// Why an item was not rendered.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RenderError {
    /// The renderer does not render this kind of item.
    #[error("{renderer} does not render {item}")]
    Unsupported {
        /// Which item.
        item: &'static str,
        /// Which renderer.
        renderer: String,
    },
    /// Rendering failed.
    #[error("render failed: {0}")]
    Failed(String),
}

harness_async_trait! {
    /// A native renderer (R25.3): renders the subset of items its capabilities cover.
    pub trait Renderer: Entity {
        /// What the renderer can show.
        fn capabilities(&self) -> &SurfaceCapabilities;
        /// Renders one item.
        async fn render(&self, item: RenderItem) -> Result<(), RenderError>;
        /// Whether `item` is within the capabilities: an MCP App needs the flag, an A2UI
        /// payload needs its catalogs; everything else is a renderer's choice (R25.3).
        fn supports(&self, item: &RenderItem) -> bool {
            match item {
                RenderItem::McpApp(_) => self.capabilities().mcp_apps,
                RenderItem::A2ui(messages) => {
                    self.capabilities().renders_a2ui(a2ui::catalogs(messages))
                }
                _ => true,
            }
        }
    }
}

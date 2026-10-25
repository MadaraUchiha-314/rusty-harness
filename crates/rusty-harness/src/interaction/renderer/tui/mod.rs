//! The terminal renderer on ratatui (R27); compiled with the `tui` feature.
//!
//! The TUI is an A2A client (R27.3): it renders the `Task`, `TaskStatusUpdateEvent` and
//! `TaskArtifactUpdateEvent` stream of a harness it talks to over the loopback URL of the
//! in-process server or `--url`, and nothing else. [`App`] is the state and the reducer
//! (events in, [`Outgoing`] requests out), [`view::draw`] paints it per the prototype
//! (`design/tui-renderer.html`: header, conversation, plan strip, composer, key hints),
//! [`form::Form`] renders an A2UI basic-catalog ask natively and turns its buttons into
//! A2UI actions, and [`run::run`] drives a real terminal. Every string shown comes from
//! the catalog (R27.8); `NO_COLOR` selects the monochrome [`Theme`].

pub mod app;
pub mod form;
pub mod run;
pub mod view;

pub use app::{App, Footer, Incoming, ModelEntry, ModelTrait, Outgoing, TuiOptions};
pub use run::{TuiError, run};
pub use view::Theme;

use std::sync::Mutex;

use crate::harness_async_trait;
use crate::interaction::renderer::{RenderError, RenderItem, Renderer};
use crate::interaction::surface::SurfaceCapabilities;
use crate::registry::{Entity, EntityId, EntityKind};

/// The TUI as a [`Renderer`] entity: items rendered into the shared [`App`] state.
pub struct Tui {
    id: EntityId,
    app: Mutex<App>,
}

impl core::fmt::Debug for Tui {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tui").finish_non_exhaustive()
    }
}

impl Tui {
    /// The renderer over `app`.
    #[must_use]
    pub fn new(app: App) -> Self {
        Self {
            id: EntityId::new("tui"),
            app: Mutex::new(app),
        }
    }

    /// The state, for drawing.
    pub fn app(&self) -> std::sync::MutexGuard<'_, App> {
        self.app
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Entity for Tui {
    fn kind(&self) -> EntityKind {
        EntityKind::Renderer
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Renderer for Tui {
        fn capabilities(&self) -> &SurfaceCapabilities {
            // The capabilities are fixed for the life of the renderer, so the lock is
            // never held across an await; the reference outlives the guard through the
            // static declaration below.
            App::surface_static()
        }
        async fn render(&self, item: RenderItem) -> Result<(), RenderError> {
            let mut app = self.app();
            if !app.supports(&item) {
                return Err(RenderError::Unsupported { item: item.name(), renderer: "tui".into() });
            }
            app.render_item(item);
            Ok(())
        }
    }
}

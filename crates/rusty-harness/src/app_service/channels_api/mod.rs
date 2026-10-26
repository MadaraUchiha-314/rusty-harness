//! The `channels/v1` extension handling over the A2A server (R18.5–6).
//!
//! A channel's question rides on the task's status message with `channels/v1` metadata;
//! on the wire, a caller who is not a member of the channel gets the state change and
//! the sub-state but not the question (abuse 2). The event log is already a
//! per-subscriber projection (`Visibility::Members`); this module applies the same rule
//! to the task snapshot `GetTask` returns and resolves a caller's memberships for the
//! stream.

use std::sync::Arc;

use crate::harness::channels::{A2UI_EXT, CHANNELS_EXT, Channel, ChannelsMeta};
use crate::harness::core_loop::persistence::{PersistenceStore, StoreError};
use crate::harness::core_loop::task::Caller;

/// The caller as a viewer of `task`: the channels it is a member of attached (R18.5).
///
/// # Errors
///
/// The store.
pub async fn viewer_for(
    store: &dyn PersistenceStore,
    task: &str,
    caller: &Caller,
) -> Result<Caller, StoreError> {
    let channels = Channel::memberships(store, task, &caller.id).await?;
    Ok(caller.clone().in_channels(channels))
}

/// Whether `caller` may read `channel` on `task`.
///
/// # Errors
///
/// The store.
pub async fn is_member(
    store: &dyn PersistenceStore,
    task: &str,
    channel: &str,
    caller: &Caller,
) -> Result<bool, StoreError> {
    Ok(match Channel::load(store, channel).await? {
        Some(c) => c.task == task && c.admits(&caller.id),
        None => false,
    })
}

/// Redacts the question on `task`'s status message when `viewer` is not a member of
/// its channel: the parts and the A2UI payload go, the state, the sub-state and a
/// `redacted` flag stay (R18.5, abuse 2).
///
/// # Errors
///
/// The store.
pub async fn redact_for_viewer(
    store: &dyn PersistenceStore,
    task: &mut a2a::Task,
    viewer: &Caller,
) -> Result<bool, StoreError> {
    let Some(message) = task.status.message.as_mut() else {
        return Ok(false);
    };
    let Some(meta) = ChannelsMeta::of(message) else {
        return Ok(false);
    };
    if meta.to.is_empty()
        || viewer.channels.contains(&meta.channel)
        || is_member(store, &task.id, &meta.channel, viewer).await?
    {
        return Ok(false);
    }
    message.parts.clear();
    if let Some(metadata) = message.metadata.as_mut() {
        metadata.remove(A2UI_EXT);
        metadata.insert(
            CHANNELS_EXT.to_owned(),
            serde_json::json!({ "channel": meta.channel, "ask": meta.ask, "question_kind": "text", "redacted": true }),
        );
    }
    message.extensions = None;
    Ok(true)
}

/// A store handle plus a caller: what the server needs per request.
#[derive(Clone)]
pub struct ChannelView {
    /// The store.
    pub store: Arc<dyn PersistenceStore>,
}

impl core::fmt::Debug for ChannelView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChannelView").finish_non_exhaustive()
    }
}

impl ChannelView {
    /// Over `store`.
    #[must_use]
    pub fn new(store: Arc<dyn PersistenceStore>) -> Self {
        Self { store }
    }

    /// [`redact_for_viewer`] on this store.
    ///
    /// # Errors
    ///
    /// The store.
    pub async fn redact(&self, task: &mut a2a::Task, viewer: &Caller) -> Result<bool, StoreError> {
        redact_for_viewer(self.store.as_ref(), task, viewer).await
    }
}

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
use crate::runtime::{HarnessStream, boxed_stream};
use futures::StreamExt as _;

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

/// The task's log as a stream for `viewer`: the Task snapshot first (redacted for a
/// non-member of a channel), then the stored events from `from` without the raw
/// `Message` entries, following live commits until the turn ends (R19.7, R19.8).
pub async fn log_stream(
    store: Arc<dyn PersistenceStore>,
    task: String,
    from: u64,
    viewer: Caller,
) -> HarnessStream<'static, Result<a2a::StreamResponse, a2a::A2AError>> {
    let first: Vec<Result<a2a::StreamResponse, a2a::A2AError>> =
        match store.a2a_task(&task, &viewer).await {
            Ok(Some(mut t)) => {
                let _ = redact_for_viewer(store.as_ref(), &mut t, &viewer).await;
                vec![Ok(a2a::StreamResponse::Task(t))]
            }
            Ok(None) => vec![Err(a2a::A2AError::task_not_found(&task))],
            Err(e) => vec![Err(a2a::A2AError::internal(e.to_string()))],
        };
    let is_message = |e: &crate::harness::core_loop::persistence::StoredEvent| {
        matches!(e.response, a2a::StreamResponse::Message(_))
    };
    let live = store.get_task(&task).await.ok().flatten().is_some_and(|r| {
        r.state == a2a::TaskState::Working || r.state == a2a::TaskState::Submitted
    });
    if !live {
        let events = store
            .events(&task, from, usize::MAX, &viewer)
            .await
            .unwrap_or_default();
        return boxed_stream(futures::stream::iter(
            first
                .into_iter()
                .chain(
                    events
                        .into_iter()
                        .filter(|e| !is_message(e))
                        .map(|e| Ok(e.response)),
                )
                .collect::<Vec<_>>(),
        ));
    }
    let tail = store
        .tail_events(&task, from, &viewer)
        .filter(move |r| futures::future::ready(!matches!(r, Ok(e) if is_message(e))))
        .map(|r| {
            r.map(|e| e.response)
                .map_err(|e| a2a::A2AError::internal(e.to_string()))
        });
    boxed_stream(futures::stream::iter(first).chain(futures::stream::unfold(
        (tail, false),
        |(mut tail, done)| async move {
            if done {
                return None;
            }
            let item = tail.next().await?;
            let stop = matches!(&item, Ok(e) if turn_ends(e));
            Some((item, (tail, stop)))
        },
    )))
}

/// Ends a tail after the event that leaves the task waiting or terminal: one turn per
/// request on the wire (R21.3).
#[must_use]
pub fn turn_ends(event: &a2a::StreamResponse) -> bool {
    match event {
        a2a::StreamResponse::StatusUpdate(u) => {
            u.status.state.is_terminal() || u.status.state == a2a::TaskState::InputRequired
        }
        a2a::StreamResponse::Task(t) => t.status.state.is_terminal(),
        _ => false,
    }
}

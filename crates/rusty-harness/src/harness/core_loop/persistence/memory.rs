//! The default store: maps under one lock, per-task subscriber lists for tails.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};

use bytes::Bytes;
use futures::StreamExt as _;
use futures::channel::mpsc;

use super::{
    BlobKey, CommitSeq, Page, PersistenceStore, Seq, StoreError, StoredEvent, TaskCommit,
    TaskQuery, new_events,
};
use crate::app_service::inbox::{EventId, EventRef, EventTarget, InboxEvent, InboxStatus};
use crate::harness::core_loop::plan::{Plan, PlanId};
use crate::harness::core_loop::state::AgentState;
use crate::harness::core_loop::task::{Caller, TaskId, TaskRecord};
use crate::harness_async_trait;
use crate::registry::{Entity, EntityId, EntityKind};
use crate::runtime::{HarnessStream, boxed_stream};

const DEFAULT_PAGE: usize = 50;

struct Tail {
    task: TaskId,
    viewer: Caller,
    sender: mpsc::UnboundedSender<StoredEvent>,
}

#[derive(Default)]
pub(crate) struct Inner {
    pub(crate) tasks: BTreeMap<TaskId, TaskRecord>,
    pub(crate) events: BTreeMap<TaskId, Vec<StoredEvent>>,
    pub(crate) event_ids: BTreeMap<TaskId, BTreeSet<EventId>>,
    pub(crate) commits: BTreeMap<TaskId, CommitSeq>,
    pub(crate) state: BTreeMap<TaskId, AgentState>,
    pub(crate) plans: BTreeMap<PlanId, Plan>,
    pub(crate) blobs: BTreeMap<BlobKey, Bytes>,
    pub(crate) inbox: BTreeMap<EventId, InboxEvent>,
    tails: Vec<Tail>,
}

/// What a commit will do, checked before anything is written (shared with the file
/// store, whose journal write sits between the check and the apply).
pub(crate) struct Checked {
    pub(crate) seq: CommitSeq,
    pub(crate) version: u64,
    pub(crate) new_event_count: usize,
}

impl Inner {
    pub(crate) fn check(&self, commit: &TaskCommit) -> Result<Checked, StoreError> {
        let id = &commit.task.id;
        let current = self.tasks.get(id);
        let actual = current.map_or(0, |t| t.version);
        if let Some(expected) = commit.expect_version
            && expected != actual
        {
            return Err(StoreError::StaleVersion {
                task: id.clone(),
                expected,
                actual,
            });
        }
        if current.is_some_and(TaskRecord::is_terminal) {
            return Err(StoreError::Terminal(id.clone()));
        }
        let existing = self.event_ids.get(id).cloned().unwrap_or_default();
        Ok(Checked {
            seq: self.commits.get(id).copied().unwrap_or(0) + 1,
            version: actual + 1,
            new_event_count: new_events(&existing, &commit.events).len(),
        })
    }

    /// Applies a checked commit; returns the events that were appended (with seqs).
    pub(crate) fn apply(&mut self, mut commit: TaskCommit, checked: &Checked) -> Vec<StoredEvent> {
        let id = commit.task.id.clone();
        let existing = self.event_ids.entry(id.clone()).or_default();
        let log = self.events.entry(id.clone()).or_default();
        let mut appended = Vec::with_capacity(checked.new_event_count);
        for event in new_events(&existing.clone(), &commit.events) {
            let mut event = event.clone();
            event.seq = log.len() as Seq + 1;
            existing.insert(event.id.clone());
            log.push(event.clone());
            appended.push(event);
        }
        commit.task.version = checked.version;
        commit.task.pending_calls = std::mem::take(&mut commit.pending_calls);
        // The applied set lives on the record (design.md § Inbox): a duplicate signal is a no-op.
        for id in &commit.inbox_applied {
            if !commit.task.applied_events.contains(id) {
                commit.task.applied_events.push(id.clone());
            }
        }
        self.tasks.insert(id.clone(), commit.task);
        self.commits.insert(id.clone(), checked.seq);
        if let Some(state) = commit.state {
            self.state.insert(id.clone(), state);
        }
        if let Some(plan) = commit.plan {
            self.plans.insert(plan.id.clone(), plan);
        }
        for blob in commit.blobs {
            self.blobs.insert(blob.key, blob.bytes);
        }
        for applied in commit.inbox_applied {
            if let Some(event) = self.inbox.get_mut(&applied) {
                event.status = InboxStatus::Applied {
                    commit: checked.seq,
                };
            }
        }
        self.tails.retain(|t| !t.sender.is_closed());
        for event in &appended {
            for tail in self.tails.iter().filter(|t| t.task == id) {
                if event.visible_to(&tail.viewer) {
                    let _ = tail.sender.unbounded_send(event.clone());
                }
            }
        }
        appended
    }

    pub(crate) fn events(
        &self,
        task: &TaskId,
        from: Seq,
        limit: usize,
        viewer: &Caller,
    ) -> Vec<StoredEvent> {
        self.events
            .get(task)
            .map(|log| {
                log.iter()
                    .filter(|e| e.seq >= from && e.visible_to(viewer))
                    .take(limit)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn list(&self, query: &TaskQuery) -> Page<TaskRecord> {
        let limit = if query.limit == 0 {
            DEFAULT_PAGE
        } else {
            query.limit
        };
        let offset: usize = query
            .after
            .as_deref()
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let mut matching: Vec<&TaskRecord> =
            self.tasks.values().filter(|t| query.matches(t)).collect();
        matching.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        let items: Vec<TaskRecord> = matching
            .iter()
            .skip(offset)
            .take(limit)
            .map(|t| (*t).clone())
            .collect();
        let next =
            (offset + items.len() < matching.len()).then(|| (offset + items.len()).to_string());
        Page { items, next }
    }

    pub(crate) fn pending_inbox(&self, task: &TaskId) -> Vec<InboxEvent> {
        let mut pending: Vec<InboxEvent> = self
            .inbox
            .values()
            .filter(|e| matches!(&e.target, EventTarget::Task(t) if t == task))
            .filter(|e| !matches!(e.status, InboxStatus::Applied { .. }))
            .cloned()
            .collect();
        pending.sort_by(|a, b| {
            a.received_at
                .cmp(&b.received_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        pending
    }
}

/// The in-memory store (R14.2): the default, on every target.
pub struct MemoryStore {
    id: EntityId,
    pub(crate) inner: Mutex<Inner>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self {
            id: EntityId::new("memory"),
            inner: Mutex::new(Inner::default()),
        }
    }
}

impl core::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("MemoryStore")
            .field("tasks", &inner.tasks.len())
            .finish_non_exhaustive()
    }
}

impl MemoryStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The tail stream over the locked inner: the replay, then the live feed unless the
    /// task is already terminal.
    pub(crate) fn tail(
        inner: &mut Inner,
        task: &TaskId,
        from: Seq,
        viewer: &Caller,
    ) -> HarnessStream<'static, Result<StoredEvent, StoreError>> {
        let replay = inner.events(task, from, usize::MAX, viewer);
        let done = replay.last().is_some_and(StoredEvent::is_final)
            || inner.tasks.get(task).is_some_and(TaskRecord::is_terminal);
        let replay = futures::stream::iter(replay.into_iter().map(Ok));
        if done {
            return boxed_stream(replay);
        }
        let (sender, receiver) = mpsc::unbounded();
        inner.tails.push(Tail {
            task: task.clone(),
            viewer: viewer.clone(),
            sender,
        });
        let live = receiver.map(Ok);
        // Ends right after the final event, without waiting for one more item.
        let stream = futures::stream::unfold(
            (replay.chain(live), false),
            |(mut inner, done)| async move {
                if done {
                    return None;
                }
                let item = inner.next().await?;
                let is_final = matches!(&item, Ok(e) if e.is_final());
                Some((item, (inner, is_final)))
            },
        );
        boxed_stream(stream)
    }
}

impl Entity for MemoryStore {
    fn kind(&self) -> EntityKind {
        EntityKind::PersistenceStore
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl PersistenceStore for MemoryStore {
        async fn commit(&self, commit: TaskCommit) -> Result<CommitSeq, StoreError> {
            let mut inner = self.lock();
            let checked = inner.check(&commit)?;
            inner.apply(commit, &checked);
            Ok(checked.seq)
        }

        async fn get_task(&self, id: &TaskId) -> Result<Option<TaskRecord>, StoreError> {
            Ok(self.lock().tasks.get(id).cloned())
        }

        async fn list_tasks(&self, query: TaskQuery) -> Result<Page<TaskRecord>, StoreError> {
            Ok(self.lock().list(&query))
        }

        async fn events(&self, task: &TaskId, from: Seq, limit: usize, viewer: &Caller) -> Result<Vec<StoredEvent>, StoreError> {
            Ok(self.lock().events(task, from, limit, viewer))
        }

        fn tail_events(&self, task: &TaskId, from: Seq, viewer: &Caller) -> HarnessStream<'static, Result<StoredEvent, StoreError>> {
            Self::tail(&mut self.lock(), task, from, viewer)
        }

        async fn put_state(&self, task: &TaskId, state: &AgentState) -> Result<(), StoreError> {
            self.lock().state.insert(task.clone(), state.clone());
            Ok(())
        }

        async fn get_state(&self, task: &TaskId) -> Result<Option<AgentState>, StoreError> {
            Ok(self.lock().state.get(task).cloned())
        }

        async fn put_blob(&self, key: &BlobKey, bytes: Bytes) -> Result<(), StoreError> {
            self.lock().blobs.insert(key.clone(), bytes);
            Ok(())
        }

        async fn get_blob(&self, key: &BlobKey) -> Result<Option<Bytes>, StoreError> {
            Ok(self.lock().blobs.get(key).cloned())
        }

        async fn put_inbox(&self, event: &InboxEvent) -> Result<EventRef, StoreError> {
            let mut inner = self.lock();
            // Idempotent on the id: a second put keeps the stored event and its status.
            inner.inbox.entry(event.id.clone()).or_insert_with(|| {
                let mut stored = event.clone();
                stored.status = InboxStatus::Received;
                stored
            });
            Ok(EventRef { id: event.id.clone(), target: event.target.clone() })
        }

        async fn pending_inbox(&self, task: &TaskId) -> Result<Vec<InboxEvent>, StoreError> {
            Ok(self.lock().pending_inbox(task))
        }

        async fn mark_inbox(&self, event: &EventRef, status: InboxStatus) -> Result<(), StoreError> {
            let mut inner = self.lock();
            let stored = inner.inbox.get_mut(&event.id).ok_or_else(|| StoreError::NoSuchEvent(event.id.clone()))?;
            if matches!(status, InboxStatus::Applied { .. }) {
                return Err(StoreError::Corrupt("Applied is written by commit, not mark_inbox".into()));
            }
            stored.status = status;
            Ok(())
        }

        async fn put_plan(&self, plan: &Plan) -> Result<(), StoreError> {
            self.lock().plans.insert(plan.id.clone(), plan.clone());
            Ok(())
        }

        async fn get_plan(&self, id: &PlanId) -> Result<Option<Plan>, StoreError> {
            Ok(self.lock().plans.get(id).cloned())
        }
    }
}

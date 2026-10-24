//! The `PersistenceStore` trait with its atomic commit, the memory and file stores
//! (R14.2–4).
//!
//! [`PersistenceStore::commit`] is the **only** write path for task state: everything in
//! one [`TaskCommit`] lands or nothing does, an event whose id already exists is skipped
//! (a retried activity re-committing the same events duplicates nothing), and
//! `expect_version` rejects a stale writer. [`MemoryStore`] is the default and works on
//! every target; [`FileStore`] (native) is a journal of commits from which every other
//! record is derived, so a crash between two writes leaves a consistent store
//! (design.md § State and persistence).

use std::collections::BTreeSet;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::HarnessError;
use crate::app_service::inbox::{EventId, EventRef, InboxEvent, InboxStatus};
use crate::harness::core_loop::plan::{Plan, PlanId, PlanPersist};
use crate::harness::core_loop::state::{AgentState, StatePersist, StateWrite};
use crate::harness::core_loop::task::{
    Attempt, Caller, ContextId, PendingCall, TaskId, TaskRecord, Version,
};
use crate::harness::hooks::{HookCx, HookDispatcher};
use crate::harness_async_trait;
use crate::registry::Entity;
use crate::runtime::HarnessStream;

mod memory;
pub use memory::MemoryStore;

#[cfg(feature = "rt-tokio")]
mod file;
#[cfg(feature = "rt-tokio")]
pub use file::FileStore;

/// A position in a task's event log, from 1.
pub type Seq = u64;
/// A commit's position in a task's journal, from 1.
pub type CommitSeq = u64;
/// A blob key (`ctx/<task>/<n>`, `turn/<task>/<n>`, … design.md § At rest).
pub type BlobKey = String;

/// Who may see an event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "channel")]
pub enum Visibility {
    /// Every participant.
    #[default]
    All,
    /// Members of a channel only (a channel's ask and its replies, R18.6).
    Members(String),
}

/// One event of a task's log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredEvent {
    /// Chosen by the writer, stable across retries: `uuid v5(task, attempt, kind,
    /// ordinal)` through [`StoredEvent::event_id`].
    pub id: EventId,
    /// Assigned by the store on commit; `0` until then.
    #[serde(default)]
    pub seq: Seq,
    /// When it happened.
    pub at: DateTime<Utc>,
    /// The attempt that produced it (design.md § attempt protocol).
    pub attempt: Attempt,
    /// Who may see it.
    #[serde(default)]
    pub visibility: Visibility,
    /// The event, in the shape A2A streams it.
    pub response: a2a::StreamResponse,
}

/// The namespace for derived event ids.
const EVENT_NS: uuid::Uuid = uuid::Uuid::from_bytes(*b"rusty-harness/ev");

impl StoredEvent {
    /// The deterministic id of the `ordinal`-th event of `kind` in `attempt` of `task`.
    #[must_use]
    pub fn event_id(task: &str, attempt: Attempt, kind: &str, ordinal: u32) -> EventId {
        let name = format!(
            "{task}/{}/{}/{kind}/{ordinal}",
            attempt.iteration, attempt.n
        );
        uuid::Uuid::new_v5(&EVENT_NS, name.as_bytes()).to_string()
    }

    /// Whether this event closes the log: a status update to a terminal state.
    #[must_use]
    pub fn is_final(&self) -> bool {
        matches!(&self.response, a2a::StreamResponse::StatusUpdate(u) if u.status.state.is_terminal())
    }

    /// Whether `viewer` may see it.
    #[must_use]
    pub fn visible_to(&self, viewer: &Caller) -> bool {
        match &self.visibility {
            Visibility::All => true,
            Visibility::Members(channel) => viewer.channels.contains(channel),
        }
    }
}

/// A blob written by a commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobWrite {
    /// The key.
    pub key: BlobKey,
    /// The bytes, base64 at rest.
    #[serde(with = "base64_bytes")]
    pub bytes: Bytes,
}

mod base64_bytes {
    use base64::Engine as _;
    use bytes::Bytes;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(bytes: &Bytes, s: S) -> Result<S::Ok, S::Error> {
        base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .serialize(s)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Bytes, D::Error> {
        let text = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map(Bytes::from)
            .map_err(serde::de::Error::custom)
    }
}

/// Everything one activity step writes, landing atomically (design.md § State and
/// persistence).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskCommit {
    /// The record after the step; its `version` is assigned by the store.
    pub task: TaskRecord,
    /// New events; ids chosen by the caller, duplicates skipped.
    #[serde(default)]
    pub events: Vec<StoredEvent>,
    /// Blobs to write.
    #[serde(default)]
    pub blobs: Vec<BlobWrite>,
    /// The state after the step, when it changed.
    #[serde(default)]
    pub state: Option<AgentState>,
    /// The plan after the step, when it changed.
    #[serde(default)]
    pub plan: Option<Plan>,
    /// Inbox events this step folded in: marked `Applied { commit }`.
    #[serde(default)]
    pub inbox_applied: Vec<EventId>,
    /// The suspended calls after the step (replaces the record's list).
    #[serde(default)]
    pub pending_calls: Vec<PendingCall>,
    /// The version the writer read; the commit is refused when the store moved on.
    #[serde(default)]
    pub expect_version: Option<Version>,
}

impl TaskCommit {
    /// A commit of the record alone.
    #[must_use]
    pub fn of(task: TaskRecord) -> Self {
        let pending_calls = task.pending_calls.clone();
        let expect_version = Some(task.version);
        Self {
            task,
            events: Vec::new(),
            blobs: Vec::new(),
            state: None,
            plan: None,
            inbox_applied: Vec::new(),
            pending_calls,
            expect_version,
        }
    }

    /// With `events`.
    #[must_use]
    pub fn with_events(mut self, events: impl IntoIterator<Item = StoredEvent>) -> Self {
        self.events.extend(events);
        self
    }

    /// Without the optimistic check (replay, repair).
    #[must_use]
    pub fn unchecked(mut self) -> Self {
        self.expect_version = None;
        self
    }
}

/// A page of a list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    /// The items.
    pub items: Vec<T>,
    /// The cursor for the next page, if any.
    pub next: Option<String>,
}

/// What to list (R12): the store applies the viewer's visibility itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskQuery {
    /// Who asks; only tasks the viewer participates in are returned.
    pub viewer: Caller,
    /// Only these states, when set.
    #[serde(default)]
    pub states: Option<Vec<a2a::TaskState>>,
    /// Only this context, when set.
    #[serde(default)]
    pub context_id: Option<ContextId>,
    /// Only non-terminal tasks (the restart scan, R14.4).
    #[serde(default)]
    pub open_only: bool,
    /// The cursor from the previous page.
    #[serde(default)]
    pub after: Option<String>,
    /// Page size; `0` means the store's default (50).
    #[serde(default)]
    pub limit: usize,
}

impl TaskQuery {
    /// Everything `viewer` may see.
    #[must_use]
    pub fn for_viewer(viewer: Caller) -> Self {
        Self {
            viewer,
            states: None,
            context_id: None,
            open_only: false,
            after: None,
            limit: 0,
        }
    }

    /// Whether `task` matches.
    #[must_use]
    pub fn matches(&self, task: &TaskRecord) -> bool {
        self.viewer.sees(task)
            && self.states.as_ref().is_none_or(|s| s.contains(&task.state))
            && self
                .context_id
                .as_ref()
                .is_none_or(|c| *c == task.context_id)
            && (!self.open_only || !task.is_terminal())
    }
}

/// A typed store error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// The writer read an older version (optimistic concurrency).
    #[error("task {task}: stale write, expected version {expected}, store has {actual}")]
    StaleVersion {
        /// The task.
        task: TaskId,
        /// What the writer expected.
        expected: Version,
        /// What the store holds.
        actual: Version,
    },
    /// The task is terminal; no further writes (R21.8).
    #[error("task {0}: terminal, no further writes")]
    Terminal(TaskId),
    /// No such inbox event.
    #[error("inbox event {0}: not found")]
    NoSuchEvent(EventId),
    /// The platform failed.
    #[error("store i/o: {0}")]
    Io(String),
    /// A record at rest does not parse.
    #[error("store corrupt: {0}")]
    Corrupt(String),
}

impl From<StoreError> for HarnessError {
    fn from(e: StoreError) -> Self {
        Self::Internal(e.to_string())
    }
}

harness_async_trait! {
    /// Where tasks live (R14.2). Every method is keyed, so a durable workflow can carry
    /// store keys instead of payloads.
    pub trait PersistenceStore: Entity {
        /// Lands `commit` atomically; the only write path for task state.
        async fn commit(&self, commit: TaskCommit) -> Result<CommitSeq, StoreError>;
        /// The record.
        async fn get_task(&self, id: &TaskId) -> Result<Option<TaskRecord>, StoreError>;
        /// The tasks matching `query`, newest first.
        async fn list_tasks(&self, query: TaskQuery) -> Result<Page<TaskRecord>, StoreError>;
        /// Up to `limit` stored events of `task` from `from`, visible to `viewer`.
        async fn events(&self, task: &TaskId, from: Seq, limit: usize, viewer: &Caller) -> Result<Vec<StoredEvent>, StoreError>;
        /// The events of `task` from `from`, then every later one as it is committed,
        /// filtered for `viewer`; ends after the final event (a per-subscriber projection).
        fn tail_events(&self, task: &TaskId, from: Seq, viewer: &Caller) -> HarnessStream<'static, Result<StoredEvent, StoreError>>;
        /// The state.
        async fn put_state(&self, task: &TaskId, state: &AgentState) -> Result<(), StoreError>;
        /// The state.
        async fn get_state(&self, task: &TaskId) -> Result<Option<AgentState>, StoreError>;
        /// A blob.
        async fn put_blob(&self, key: &BlobKey, bytes: Bytes) -> Result<(), StoreError>;
        /// A blob.
        async fn get_blob(&self, key: &BlobKey) -> Result<Option<Bytes>, StoreError>;
        /// Stores an accepted inbox event as `Received` (R20.3).
        async fn put_inbox(&self, event: &InboxEvent) -> Result<EventRef, StoreError>;
        /// The inbox events of `task` not yet applied, oldest first.
        async fn pending_inbox(&self, task: &TaskId) -> Result<Vec<InboxEvent>, StoreError>;
        /// Moves an inbox event to `Received` or `Signalled`; `Applied` is written by
        /// `commit` only.
        async fn mark_inbox(&self, event: &EventRef, status: InboxStatus) -> Result<(), StoreError>;
        /// The plan.
        async fn put_plan(&self, plan: &Plan) -> Result<(), StoreError>;
        /// The plan.
        async fn get_plan(&self, id: &PlanId) -> Result<Option<Plan>, StoreError>;

        /// The A2A projection of `id`: the record with its message history, bounded by
        /// the record's `history_len` (R12.2, R14.4).
        async fn a2a_task(&self, id: &TaskId, viewer: &Caller) -> Result<Option<a2a::Task>, StoreError> {
            let Some(task) = self.get_task(id).await? else { return Ok(None) };
            let events = self.events(id, 1, usize::MAX, viewer).await?;
            let mut history: Vec<a2a::Message> = events
                .into_iter()
                .filter_map(|e| match e.response {
                    a2a::StreamResponse::Message(m) => Some(m),
                    _ => None,
                })
                .collect();
            if task.history_len > 0 && history.len() as u64 > task.history_len {
                let skip = history.len() - usize::try_from(task.history_len).unwrap_or(usize::MAX);
                history.drain(..skip);
            }
            Ok(Some(task.to_a2a(history)))
        }
    }
}

/// Persists `plan` through `plan.persist` hooks (R14.3), then the store.
///
/// # Errors
///
/// A veto, a hook failure or a store error.
pub async fn persist_plan(
    store: &dyn PersistenceStore,
    dispatcher: &HookDispatcher,
    cx: &HookCx,
    plan: Plan,
) -> Result<(), HarnessError> {
    dispatcher
        .run::<PlanPersist, _, _>(cx, plan, |plan| async move {
            store.put_plan(&plan).await.map_err(HarnessError::from)
        })
        .await
}

/// Persists a state write through `state.persist` hooks (R14.2), then the store.
///
/// # Errors
///
/// A veto, a hook failure or a store error.
pub async fn persist_state(
    store: &dyn PersistenceStore,
    dispatcher: &HookDispatcher,
    cx: &HookCx,
    write: StateWrite,
) -> Result<(), HarnessError> {
    dispatcher
        .run::<StatePersist, _, _>(cx, write, |write| async move {
            store
                .put_state(&write.task, &write.state)
                .await
                .map_err(HarnessError::from)
        })
        .await
}

/// The event ids already in a log, for the dedupe every store applies.
pub(crate) fn new_events<'a>(
    existing: &BTreeSet<EventId>,
    events: &'a [StoredEvent],
) -> Vec<&'a StoredEvent> {
    let mut seen = existing.clone();
    events
        .iter()
        .filter(|e| seen.insert(e.id.clone()))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        BlobWrite, MemoryStore, PersistenceStore, StoreError, StoredEvent, TaskCommit, TaskQuery,
        Visibility, persist_plan,
    };
    use crate::app_service::inbox::{DeliveryKind, EventTarget, InboxEvent, InboxStatus};
    use crate::harness::core_loop::plan::{Plan, PlanPersist, Step};
    use crate::harness::core_loop::state::AgentState;
    use crate::harness::core_loop::task::{
        Attempt, Caller, Role, TaskMode, TaskRecord, Transition,
    };
    use crate::harness::hooks::{Hook, HookCx, HookDispatcher, HookError, HookOrder, PostHook};
    use crate::harness_async_trait;
    use crate::host::Host;
    use crate::registry::{EntityId, Registry};
    use a2a::TaskState;
    use chrono::{TimeZone, Utc};
    use futures::StreamExt as _;
    use std::sync::{Arc, Mutex};

    pub(crate) fn at(n: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, n).unwrap()
    }

    pub(crate) fn task(id: &str) -> TaskRecord {
        TaskRecord::new(
            crate::harness::core_loop::task::tests::draft(id, TaskMode::LongRunning),
            at(0),
        )
    }

    pub(crate) fn message(task: &str, attempt: Attempt, ordinal: u32, text: &str) -> StoredEvent {
        let mut m = a2a::Message::new(a2a::Role::Agent, vec![a2a::Part::text(text)]);
        m.task_id = Some(task.to_owned());
        StoredEvent {
            id: StoredEvent::event_id(task, attempt, "message", ordinal),
            seq: 0,
            at: at(ordinal),
            attempt,
            visibility: Visibility::All,
            response: a2a::StreamResponse::Message(m),
        }
    }

    fn status(task: &TaskRecord) -> StoredEvent {
        StoredEvent {
            id: StoredEvent::event_id(&task.id, Attempt::default(), "status", 0),
            seq: 0,
            at: task.updated_at,
            attempt: Attempt::default(),
            visibility: Visibility::All,
            response: a2a::StreamResponse::StatusUpdate(task.status_update()),
        }
    }

    /// The scenarios every store must pass; the file store runs them too.
    #[allow(clippy::too_many_lines)]
    pub(crate) async fn exercise(store: &dyn PersistenceStore) {
        let viewer = Caller::human("u1", [Role::Reporter]);
        let mut t = task("t1");
        t.transition(&Transition::Start, at(1)).unwrap();
        let a1 = Attempt { iteration: 1, n: 1 };
        let first = TaskCommit::of(t.clone())
            .with_events([message("t1", a1, 1, "hello"), message("t1", a1, 2, "world")]);
        assert_eq!(store.commit(first.clone()).await.unwrap(), 1);
        let stored = store.get_task(&"t1".into()).await.unwrap().unwrap();
        assert_eq!(stored.version, 1);
        assert_eq!(stored.state, TaskState::Working);

        // Idempotent on event ids: a retried activity re-committing the same events
        // duplicates nothing, and the version still advances.
        let mut again = first.clone();
        again.task = stored.clone();
        again.expect_version = Some(1);
        again.events.push(message("t1", a1, 3, "again"));
        assert_eq!(store.commit(again).await.unwrap(), 2);
        let events = store.events(&"t1".into(), 1, 100, &viewer).await.unwrap();
        assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(events.len(), 3, "no duplicate");

        // A stale writer is refused and changes nothing.
        let mut old_writer = TaskCommit::of(stored.clone());
        old_writer.expect_version = Some(1);
        old_writer.events.push(message("t1", a1, 9, "stale"));
        let err = store.commit(old_writer).await.unwrap_err();
        assert_eq!(
            err,
            StoreError::StaleVersion {
                task: "t1".into(),
                expected: 1,
                actual: 2
            }
        );
        assert_eq!(
            store
                .events(&"t1".into(), 1, 100, &viewer)
                .await
                .unwrap()
                .len(),
            3
        );

        // State, plan, blobs and inbox marks ride in the same commit.
        let mut state = AgentState::new();
        state.set("n", 1).unwrap();
        let mut plan = Plan::new("p1", "t1");
        plan.add_step(Step::new("s1", "one", "")).unwrap();
        let inbox = InboxEvent {
            id: "in-1".into(),
            target: EventTarget::Task("t1".into()),
            kind: a2a::StreamResponse::Message(a2a::Message::new(
                a2a::Role::User,
                vec![a2a::Part::text("hi")],
            )),
            status: InboxStatus::Received,
            sender: viewer.clone(),
            received_at: at(5),
            delivery: DeliveryKind::Interrupt,
            extensions: vec![],
            traceparent: None,
        };
        store.put_inbox(&inbox).await.unwrap();
        assert_eq!(store.pending_inbox(&"t1".into()).await.unwrap().len(), 1);
        let mut current = store.get_task(&"t1".into()).await.unwrap().unwrap();
        current.plan = Some("p1".into());
        let mut c3 = TaskCommit::of(current);
        c3.state = Some(state.clone());
        c3.plan = Some(plan.clone());
        c3.blobs.push(BlobWrite {
            key: "ctx/t1/1".into(),
            bytes: bytes::Bytes::from_static(b"{\"ctx\":1}"),
        });
        c3.inbox_applied.push("in-1".into());
        assert_eq!(store.commit(c3).await.unwrap(), 3);
        assert_eq!(store.get_state(&"t1".into()).await.unwrap(), Some(state));
        assert_eq!(store.get_plan(&"p1".into()).await.unwrap(), Some(plan));
        assert_eq!(
            store.get_blob(&"ctx/t1/1".into()).await.unwrap().as_deref(),
            Some(&b"{\"ctx\":1}"[..])
        );
        assert!(
            store.pending_inbox(&"t1".into()).await.unwrap().is_empty(),
            "applied by the commit"
        );

        // The viewer projection: a channel's event is invisible to a non-member.
        let mut current = store.get_task(&"t1".into()).await.unwrap().unwrap();
        let mut secret = message("t1", a1, 4, "members only");
        secret.visibility = Visibility::Members("ch-1".into());
        current.transition(&Transition::Delivered, at(6)).unwrap();
        let c4 = TaskCommit::of(current.clone()).with_events([secret, status(&current)]);
        assert_eq!(store.commit(c4).await.unwrap(), 4);
        let outsider = store.events(&"t1".into(), 1, 100, &viewer).await.unwrap();
        assert_eq!(outsider.len(), 4);
        let member = Caller::human("u1", [Role::Reporter]).in_channels(["ch-1".into()]);
        assert_eq!(
            store
                .events(&"t1".into(), 1, 100, &member)
                .await
                .unwrap()
                .len(),
            5
        );

        // The A2A projection carries the history and the extension metadata.
        let wire = store
            .a2a_task(&"t1".into(), &viewer)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(wire.status.state, TaskState::InputRequired);
        assert_eq!(wire.history.as_ref().map(Vec::len), Some(3));
        assert!(
            wire.metadata
                .unwrap()
                .contains_key(crate::harness::core_loop::task::TASK_EXT)
        );

        // Listing applies the viewer.
        let page = store
            .list_tasks(TaskQuery::for_viewer(viewer.clone()))
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        let page = store
            .list_tasks(TaskQuery::for_viewer(Caller::human("nobody", [])))
            .await
            .unwrap();
        assert_eq!(page.items.len(), 0);

        // Terminal: the stop path's last commit lands, nothing after it.
        let mut current = store.get_task(&"t1".into()).await.unwrap().unwrap();
        current.transition(&Transition::Complete, at(7)).unwrap();
        let c5 = TaskCommit::of(current.clone()).with_events([status(&current)]);
        assert_eq!(store.commit(c5).await.unwrap(), 5);
        let mut after = TaskCommit::of(store.get_task(&"t1".into()).await.unwrap().unwrap());
        after.events.push(message("t1", a1, 10, "late"));
        assert_eq!(
            store.commit(after).await.unwrap_err(),
            StoreError::Terminal("t1".into())
        );
        let mut q = TaskQuery::for_viewer(viewer.clone());
        q.open_only = true;
        assert_eq!(store.list_tasks(q).await.unwrap().items.len(), 0);
    }

    #[tokio::test]
    async fn the_memory_store_commits_atomically_and_idempotently() {
        let store = MemoryStore::default();
        exercise(&store).await;
    }

    #[tokio::test]
    async fn a_tail_replays_then_follows_live_commits_and_ends_at_the_final_event() {
        let store = Arc::new(MemoryStore::default());
        let viewer = Caller::human("u1", [Role::Reporter]);
        let mut t = task("t2");
        t.transition(&Transition::Start, at(1)).unwrap();
        let a1 = Attempt { iteration: 1, n: 1 };
        store
            .commit(TaskCommit::of(t.clone()).with_events([message("t2", a1, 1, "one")]))
            .await
            .unwrap();
        let mut tail = store.tail_events(&"t2".into(), 1, &viewer);
        let first = tail.next().await.unwrap().unwrap();
        assert_eq!(first.seq, 1);
        let mut current = store.get_task(&"t2".into()).await.unwrap().unwrap();
        let mut hidden = message("t2", a1, 2, "hidden");
        hidden.visibility = Visibility::Members("ch".into());
        current.transition(&Transition::Complete, at(3)).unwrap();
        store
            .commit(TaskCommit::of(current.clone()).with_events([
                hidden,
                message("t2", a1, 3, "three"),
                status(&current),
            ]))
            .await
            .unwrap();
        let rest: Vec<_> = tail.map(|e| e.unwrap().seq).collect().await;
        assert_eq!(
            rest,
            [3, 4],
            "the hidden event is skipped, the final status ends the tail"
        );
        // A late subscriber on a terminal task gets the replay only.
        let late: Vec<_> = store
            .tail_events(&"t2".into(), 3, &viewer)
            .map(|e| e.unwrap().seq)
            .collect()
            .await;
        assert_eq!(late, [3, 4]);
    }

    #[tokio::test]
    async fn plan_persist_hooks_fire_on_every_put_plan() {
        struct Mirror(Mutex<Vec<String>>);
        harness_async_trait! {
            impl PostHook<PlanPersist> for Mirror {
                async fn post(&self, _cx: &HookCx, plan: &Plan, output: ()) -> Result<(), HookError> {
                    self.0.lock().unwrap().push(format!("{}@{}", plan.id, plan.version));
                    Ok(output)
                }
            }
        }
        let mirror = Arc::new(Mirror(Mutex::new(Vec::new())));
        let d = HookDispatcher::new();
        d.register::<PlanPersist>(
            EntityId::new("mirror"),
            Hook::Post(mirror.clone()),
            HookOrder::default(),
        );
        let cx = HookCx::new(Arc::new(Registry::new()), Arc::new(Host::testing()));
        let store = MemoryStore::default();
        let mut plan = Plan::new("p9", "t9");
        plan.add_step(Step::new("a", "a", "")).unwrap();
        persist_plan(&store, &d, &cx, plan.clone()).await.unwrap();
        assert_eq!(store.get_plan(&"p9".into()).await.unwrap(), Some(plan));
        assert_eq!(mirror.0.lock().unwrap().as_slice(), ["p9@1"]);
    }
}

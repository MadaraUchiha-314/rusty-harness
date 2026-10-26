//! The file store (native): a journal of commits under `commits/<task>/<seq>.json`, each
//! written by temp file + atomic rename, with the task, event, state and plan records
//! materialized beside it and rebuilt from the journal on open when they are behind.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::Mutex;

use super::memory::MemoryStore;
use super::{
    BlobKey, CommitSeq, Page, PersistenceStore, Seq, StoreError, StoredEvent, TaskCommit, TaskQuery,
};
use crate::app_service::inbox::{EventRef, InboxEvent, InboxStatus};
use crate::harness::core_loop::plan::{Plan, PlanId};
use crate::harness::core_loop::state::AgentState;
use crate::harness::core_loop::task::{Caller, TaskId, TaskRecord};
use crate::harness_async_trait;
use crate::registry::{Entity, EntityId, EntityKind};
use crate::runtime::HarnessStream;

/// The file store (R14.2): the journal is authoritative, everything else is a view.
pub struct FileStore {
    id: EntityId,
    root: PathBuf,
    index: Arc<MemoryStore>,
    write: Mutex<()>,
}

impl core::fmt::Debug for FileStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileStore")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

fn io(path: &Path, e: &std::io::Error) -> StoreError {
    StoreError::Io(format!("{}: {}", path.display(), e.kind()))
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| StoreError::Io(format!("{}: no parent", path.display())))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| io(parent, &e))?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| io(&tmp, &e))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| io(path, &e))
}

async fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, StoreError> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| StoreError::Corrupt(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(path, &e)),
    }
}

async fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec(value).map_err(|e| StoreError::Corrupt(e.to_string()))?;
    write_atomic(path, &bytes).await
}

/// The entries of `dir`, sorted, skipping temp files; empty when the dir is missing.
///
/// Never deletes: a `.tmp` here may be a write in flight, in this process or in another
/// one on the same directory (R22.5), whose rename would then fail and lose the commit.
async fn entries(dir: &Path) -> Result<Vec<PathBuf>, StoreError> {
    let mut out = Vec::new();
    let mut rd = match tokio::fs::read_dir(dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(io(dir, &e)),
    };
    while let Some(entry) = rd.next_entry().await.map_err(|e| io(dir, &e))? {
        let path = entry.path();
        if path.extension().is_some_and(|x| x == "tmp") {
            continue;
        }
        out.push(path);
    }
    out.sort();
    Ok(out)
}

/// How old a temp file must be before the replay treats it as a crash's leftover.
const STALE_TMP: std::time::Duration = std::time::Duration::from_secs(600);

/// Removes the temp files under `dir` (one level) older than [`STALE_TMP`]: writes a
/// crash interrupted, which no live writer can still be renaming.
async fn sweep_stale_tmp(dir: &Path) {
    let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        let path = entry.path();
        if path.is_dir() {
            Box::pin(sweep_stale_tmp(&path)).await;
            continue;
        }
        if path.extension().is_none_or(|x| x != "tmp") {
            continue;
        }
        let stale = entry
            .metadata()
            .await
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= STALE_TMP);
        if stale {
            let _ = tokio::fs::remove_file(&path).await;
        }
    }
}

fn safe_key(key: &str) -> Result<PathBuf, StoreError> {
    let path = Path::new(key);
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(StoreError::Io(format!(
            "blob key {key}: not a relative path"
        )));
    }
    Ok(path.to_path_buf())
}

impl FileStore {
    /// Opens (or creates) the store under `root`, replaying the journal into the index
    /// and repairing any materialized record the journal is ahead of.
    ///
    /// # Errors
    ///
    /// [`StoreError::Io`] or [`StoreError::Corrupt`].
    pub async fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root: PathBuf = root.into();
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|e| io(&root, &e))?;
        let store = Self {
            id: EntityId::new("file"),
            root,
            index: Arc::new(MemoryStore::new()),
            write: Mutex::new(()),
        };
        store.replay().await?;
        Ok(store)
    }

    /// The directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn task_path(&self, id: &str) -> PathBuf {
        self.root.join("tasks").join(format!("{id}.json"))
    }

    fn commit_path(&self, task: &str, seq: CommitSeq) -> PathBuf {
        self.root
            .join("commits")
            .join(task)
            .join(format!("{seq:08}.json"))
    }

    fn event_path(&self, task: &str, seq: Seq) -> PathBuf {
        self.root
            .join("events")
            .join(task)
            .join(format!("{seq:08}.json"))
    }

    async fn replay(&self) -> Result<(), StoreError> {
        sweep_stale_tmp(&self.root).await;
        // Inbox first, so a commit's `inbox_applied` finds its events.
        for path in entries(&self.root.join("inbox")).await? {
            if let Some(event) = read_json::<InboxEvent>(&path).await? {
                self.index.lock().inbox.insert(event.id.clone(), event);
            }
        }
        let mut behind: Vec<TaskId> = Vec::new();
        for dir in entries(&self.root.join("commits")).await? {
            let Some(task) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
                continue;
            };
            for path in entries(&dir).await? {
                let commit = read_json::<TaskCommit>(&path)
                    .await?
                    .ok_or_else(|| StoreError::Corrupt(path.display().to_string()))?
                    .unchecked();
                let mut index = self.index.lock();
                let checked = index.check(&commit)?;
                index.apply(commit, &checked);
            }
            let journal_version = self.index.lock().tasks.get(&task).map_or(0, |t| t.version);
            let materialized = read_json::<TaskRecord>(&self.task_path(&task)).await?;
            if materialized.is_none_or(|t| t.version < journal_version) {
                behind.push(task);
            }
        }
        for task in behind {
            self.materialize_all(&task).await?;
        }
        // A state or plan put outside a commit is at least as new as the journal's copy.
        for path in entries(&self.root.join("state")).await? {
            if let (Some(stem), Some(state)) = (
                path.file_stem().and_then(|s| s.to_str()),
                read_json::<AgentState>(&path).await?,
            ) {
                self.index.lock().state.insert(stem.to_owned(), state);
            }
        }
        for path in entries(&self.root.join("plans")).await? {
            if let Some(plan) = read_json::<Plan>(&path).await? {
                self.index.lock().plans.insert(plan.id.clone(), plan);
            }
        }
        // Blobs live on disk; the replay's copies are written if missing, then dropped.
        let blobs = std::mem::take(&mut self.index.lock().blobs);
        for (key, bytes) in blobs {
            let path = self.root.join("blobs").join(safe_key(&key)?);
            if tokio::fs::metadata(&path).await.is_err() {
                write_atomic(&path, &bytes).await?;
            }
        }
        Ok(())
    }

    /// Rewrites every view of `task` from the index (which the journal built).
    async fn materialize_all(&self, task: &TaskId) -> Result<(), StoreError> {
        let (record, events, state, plan, inbox) = {
            let index = self.index.lock();
            (
                index.tasks.get(task).cloned(),
                index.events.get(task).cloned().unwrap_or_default(),
                index.state.get(task).cloned(),
                index
                    .tasks
                    .get(task)
                    .and_then(|t| t.plan.clone())
                    .and_then(|p| index.plans.get(&p).cloned()),
                index.pending_inbox(task),
            )
        };
        let Some(record) = record else { return Ok(()) };
        for event in &events {
            let path = self.event_path(task, event.seq);
            if tokio::fs::metadata(&path).await.is_err() {
                write_json(&path, event).await?;
            }
        }
        if let Some(state) = state {
            write_json(
                &self.root.join("state").join(format!("{task}.json")),
                &state,
            )
            .await?;
        }
        if let Some(plan) = plan {
            write_json(
                &self.root.join("plans").join(format!("{}.json", plan.id)),
                &plan,
            )
            .await?;
        }
        for event in inbox {
            write_json(
                &self.root.join("inbox").join(format!("{}.json", event.id)),
                &event,
            )
            .await?;
        }
        write_json(&self.task_path(task), &record).await
    }

    /// Writes the views a commit changed.
    async fn materialize(
        &self,
        commit: &TaskCommit,
        appended: &[StoredEvent],
    ) -> Result<(), StoreError> {
        let task = &commit.task.id;
        for event in appended {
            write_json(&self.event_path(task, event.seq), event).await?;
        }
        for blob in &commit.blobs {
            write_atomic(
                &self.root.join("blobs").join(safe_key(&blob.key)?),
                &blob.bytes,
            )
            .await?;
        }
        if let Some(state) = &commit.state {
            write_json(&self.root.join("state").join(format!("{task}.json")), state).await?;
        }
        if let Some(plan) = &commit.plan {
            write_json(
                &self.root.join("plans").join(format!("{}.json", plan.id)),
                plan,
            )
            .await?;
        }
        for id in &commit.inbox_applied {
            let event = self.index.lock().inbox.get(id).cloned();
            if let Some(event) = event {
                write_json(&self.root.join("inbox").join(format!("{id}.json")), &event).await?;
            }
        }
        let record = self.index.lock().tasks.get(task).cloned();
        if let Some(record) = record {
            write_json(&self.task_path(task), &record).await?;
        }
        Ok(())
    }
}

/// Brings `index` up to date with what other processes wrote for `task` under `root`:
/// the journal entries after the last one applied here, and the inbox files. Two workers
/// on one directory (R22.5) see each other's commits through this.
async fn refresh_task(root: &Path, index: &MemoryStore, task: &TaskId) -> Result<(), StoreError> {
    let known = index.lock().commits.get(task).copied().unwrap_or(0);
    let dir = root.join("commits").join(task);
    for path in entries(&dir).await? {
        let seq: CommitSeq = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if seq <= known {
            continue;
        }
        let commit = read_json::<TaskCommit>(&path)
            .await?
            .ok_or_else(|| StoreError::Corrupt(path.display().to_string()))?
            .unchecked();
        let mut guard = index.lock();
        if guard.commits.get(task).copied().unwrap_or(0) >= seq {
            continue; // another refresh got there first
        }
        let checked = guard.check(&commit)?;
        guard.apply(commit, &checked);
        guard.blobs.clear();
    }
    refresh_inbox(root, index).await
}

/// Re-reads the inbox files: an event another process accepted, or moved on.
async fn refresh_inbox(root: &Path, index: &MemoryStore) -> Result<(), StoreError> {
    for path in entries(&root.join("inbox")).await? {
        if let Some(event) = read_json::<InboxEvent>(&path).await? {
            let mut guard = index.lock();
            let newer = match guard.inbox.get(&event.id) {
                None => true,
                Some(known) => rank(&event.status) > rank(&known.status),
            };
            if newer {
                guard.inbox.insert(event.id.clone(), event);
            }
        }
    }
    Ok(())
}

impl FileStore {
    async fn refresh_task(&self, task: &TaskId) -> Result<(), StoreError> {
        refresh_task(&self.root, &self.index, task).await
    }

    async fn refresh_inbox(&self) -> Result<(), StoreError> {
        refresh_inbox(&self.root, &self.index).await
    }

    /// Every task the journal knows, refreshed.
    async fn refresh_all(&self) -> Result<(), StoreError> {
        for dir in entries(&self.root.join("commits")).await? {
            if let Some(task) = dir.file_name().and_then(|n| n.to_str()) {
                self.refresh_task(&task.to_owned()).await?;
            }
        }
        Ok(())
    }

    /// Keeps a live tail fed while it is open: another process's commits reach the index
    /// (and so the tail) by refreshing the journal every 100 ms. Only with a tokio
    /// runtime; in-process commits notify the tail directly in any case.
    fn follow_from_disk(&self, task: &TaskId) -> Option<AbortOnDrop> {
        let handle = tokio::runtime::Handle::try_current().ok()?;
        let root = self.root.clone();
        let index = Arc::clone(&self.index);
        let task = task.clone();
        let join = handle.spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                if refresh_task(&root, &index, &task).await.is_err() {
                    break;
                }
                let terminal = index
                    .lock()
                    .tasks
                    .get(&task)
                    .is_some_and(TaskRecord::is_terminal);
                if terminal {
                    break;
                }
            }
        });
        Some(AbortOnDrop(join.abort_handle()))
    }
}

/// Stops the disk follower when the tail that owns it is dropped.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// How far an inbox event has moved; disk wins when it is further along.
fn rank(status: &InboxStatus) -> u8 {
    match status {
        InboxStatus::Received => 0,
        InboxStatus::Signalled { .. } => 1,
        InboxStatus::Applied { .. } => 2,
    }
}

impl Entity for FileStore {
    fn kind(&self) -> EntityKind {
        EntityKind::PersistenceStore
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl PersistenceStore for FileStore {
        async fn commit(&self, commit: TaskCommit) -> Result<CommitSeq, StoreError> {
            let _serial = self.write.lock().await;
            self.refresh_task(&commit.task.id).await?;
            let checked = self.index.lock().check(&commit)?;
            // 1. The journal entry: once this rename lands, the commit has happened.
            write_json(&self.commit_path(&commit.task.id, checked.seq), &commit).await?;
            // 2. The index, then the views; a crash here is repaired on open.
            let appended = {
                let mut index = self.index.lock();
                let appended = index.apply(commit.clone(), &checked);
                index.blobs.clear();
                appended
            };
            self.materialize(&commit, &appended).await?;
            Ok(checked.seq)
        }

        async fn get_task(&self, id: &TaskId) -> Result<Option<TaskRecord>, StoreError> {
            self.refresh_task(id).await?;
            self.index.get_task(id).await
        }

        async fn list_tasks(&self, query: TaskQuery) -> Result<Page<TaskRecord>, StoreError> {
            self.refresh_all().await?;
            self.index.list_tasks(query).await
        }

        async fn events(&self, task: &TaskId, from: Seq, limit: usize, viewer: &Caller) -> Result<Vec<StoredEvent>, StoreError> {
            self.refresh_task(task).await?;
            self.index.events(task, from, limit, viewer).await
        }

        fn tail_events(&self, task: &TaskId, from: Seq, viewer: &Caller) -> HarnessStream<'static, Result<StoredEvent, StoreError>> {
            use futures::StreamExt as _;
            let follower = self.follow_from_disk(task);
            Box::pin(self.index.tail_events(task, from, viewer).map(move |item| {
                let _keep = &follower;
                item
            }))
        }

        async fn put_state(&self, task: &TaskId, state: &AgentState) -> Result<(), StoreError> {
            write_json(&self.root.join("state").join(format!("{task}.json")), state).await?;
            self.index.put_state(task, state).await
        }

        async fn get_state(&self, task: &TaskId) -> Result<Option<AgentState>, StoreError> {
            self.index.get_state(task).await
        }

        async fn put_blob(&self, key: &BlobKey, bytes: Bytes) -> Result<(), StoreError> {
            write_atomic(&self.root.join("blobs").join(safe_key(key)?), &bytes).await
        }

        async fn get_blob(&self, key: &BlobKey) -> Result<Option<Bytes>, StoreError> {
            let path = self.root.join("blobs").join(safe_key(key)?);
            match tokio::fs::read(&path).await {
                Ok(bytes) => Ok(Some(Bytes::from(bytes))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(io(&path, &e)),
            }
        }

        async fn put_inbox(&self, event: &InboxEvent) -> Result<EventRef, StoreError> {
            if self.index.lock().inbox.contains_key(&event.id) {
                return Ok(EventRef { id: event.id.clone(), target: event.target.clone() });
            }
            let mut stored = event.clone();
            stored.status = InboxStatus::Received;
            write_json(&self.root.join("inbox").join(format!("{}.json", event.id)), &stored).await?;
            self.index.put_inbox(event).await
        }

        async fn pending_inbox(&self, task: &TaskId) -> Result<Vec<InboxEvent>, StoreError> {
            self.refresh_task(task).await?;
            self.index.pending_inbox(task).await
        }

        async fn mark_inbox(&self, event: &EventRef, status: InboxStatus) -> Result<(), StoreError> {
            self.refresh_inbox().await?;
            self.index.mark_inbox(event, status).await?;
            let stored = self.index.lock().inbox.get(&event.id).cloned();
            if let Some(stored) = stored {
                write_json(&self.root.join("inbox").join(format!("{}.json", event.id)), &stored).await?;
            }
            Ok(())
        }

        async fn put_plan(&self, plan: &Plan) -> Result<(), StoreError> {
            write_json(&self.root.join("plans").join(format!("{}.json", plan.id)), plan).await?;
            self.index.put_plan(plan).await
        }

        async fn get_plan(&self, id: &PlanId) -> Result<Option<Plan>, StoreError> {
            self.index.get_plan(id).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FileStore;
    use crate::harness::core_loop::persistence::tests::{at, exercise, message, task};
    use crate::harness::core_loop::persistence::{PersistenceStore, TaskCommit};
    use crate::harness::core_loop::task::{Attempt, Caller, Role, Transition};

    /// A write in flight (`<seq>.json.tmp` before its rename) survives a concurrent
    /// listing; only a temp file older than `STALE_TMP` is swept, and only by the replay.
    #[tokio::test]
    async fn a_listing_never_deletes_a_write_in_flight() {
        let dir = fresh("in-flight").join("commits").join("t1");
        std::fs::create_dir_all(&dir).unwrap();
        let in_flight = dir.join("00000004.json.tmp");
        std::fs::write(&in_flight, b"{}").unwrap();
        let listed = super::entries(&dir).await.unwrap();
        assert!(listed.is_empty(), "temp files are not entries: {listed:?}");
        assert!(in_flight.exists(), "the writer's rename can still land");

        super::sweep_stale_tmp(dir.parent().unwrap()).await;
        assert!(in_flight.exists(), "a fresh temp file is not swept");
        let old =
            std::time::SystemTime::now() - super::STALE_TMP - std::time::Duration::from_secs(1);
        std::fs::File::options()
            .write(true)
            .open(&in_flight)
            .unwrap()
            .set_modified(old)
            .unwrap();
        super::sweep_stale_tmp(dir.parent().unwrap()).await;
        assert!(
            !in_flight.exists(),
            "a crash's leftover is swept by the replay"
        );
    }

    fn fresh(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rh-file-store-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn the_file_store_passes_the_store_scenarios_and_survives_a_reopen() {
        let dir = fresh("exercise");
        let store = FileStore::open(&dir).await.unwrap();
        exercise(&store).await;
        drop(store);
        let reopened = FileStore::open(&dir).await.unwrap();
        let viewer = Caller::human("u1", [Role::Reporter]);
        let wire = reopened
            .a2a_task(&"t1".into(), &viewer)
            .await
            .unwrap()
            .expect("after a restart GetTask returns the full task");
        assert_eq!(wire.status.state, a2a::TaskState::Completed);
        assert_eq!(wire.history.map(|h| h.len()), Some(3));
        assert_eq!(
            reopened
                .get_task(&"t1".into())
                .await
                .unwrap()
                .unwrap()
                .version,
            5
        );
        assert!(reopened.get_state(&"t1".into()).await.unwrap().is_some());
        assert!(reopened.get_plan(&"p1".into()).await.unwrap().is_some());
        assert_eq!(
            reopened
                .get_blob(&"ctx/t1/1".into())
                .await
                .unwrap()
                .as_deref(),
            Some(&b"{\"ctx\":1}"[..])
        );
        assert!(
            reopened.get_blob(&"../etc/passwd".into()).await.is_err(),
            "keys stay under the root"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_store_whose_materialization_is_behind_is_repaired_on_open() {
        let dir = fresh("repair");
        let store = FileStore::open(&dir).await.unwrap();
        let mut t = task("t3");
        t.transition(&Transition::Start, at(1)).unwrap();
        let a1 = Attempt { iteration: 1, n: 1 };
        store
            .commit(TaskCommit::of(t.clone()).with_events([message("t3", a1, 1, "one")]))
            .await
            .unwrap();
        let current = store.get_task(&"t3".into()).await.unwrap().unwrap();
        store
            .commit(TaskCommit::of(current).with_events([message("t3", a1, 2, "two")]))
            .await
            .unwrap();
        drop(store);
        // Simulate a crash between the journal write and the views: the second
        // commit's event file and the task record never landed, and a temp file was left.
        std::fs::remove_file(dir.join("events/t3/00000002.json")).unwrap();
        std::fs::remove_file(dir.join("tasks/t3.json")).unwrap();
        std::fs::write(dir.join("tasks/t3.json.tmp"), b"{partial").unwrap();
        let reopened = FileStore::open(&dir).await.unwrap();
        let record = reopened.get_task(&"t3".into()).await.unwrap().unwrap();
        assert_eq!(record.version, 2);
        let viewer = Caller::human("u1", [Role::Reporter]);
        assert_eq!(
            reopened
                .events(&"t3".into(), 1, 10, &viewer)
                .await
                .unwrap()
                .len(),
            2
        );
        assert!(
            dir.join("events/t3/00000002.json").exists(),
            "rebuilt from the journal"
        );
        assert!(dir.join("tasks/t3.json").exists());
        assert!(
            !dir.join("tasks/t3.json.tmp").exists(),
            "the interrupted write is gone"
        );
        // And the store keeps going from where the journal says it is.
        let current = reopened.get_task(&"t3".into()).await.unwrap().unwrap();
        assert_eq!(
            reopened
                .commit(TaskCommit::of(current).with_events([message("t3", a1, 3, "three")]))
                .await
                .unwrap(),
            3
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

//! Feature: Persistence
//! Requirement: docs/specs/issue-3/requirements.md#R14
//!
//! T13 of the testing plan, the store half: a child process commits to a `FileStore`
//! as fast as it can and is killed with SIGKILL mid-journal; the parent reopens the
//! directory and checks that the journal, the index and the views agree.
#![cfg(feature = "rt-tokio")]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rusty_harness::harness::core_loop::persistence::{
    BlobWrite, FileStore, PersistenceStore, StoredEvent, TaskCommit, Visibility,
};
use rusty_harness::harness::core_loop::task::{
    Attempt, Caller, Participant, Role, TaskDraft, TaskMode, TaskRecord, Transition,
};

const DIR_VAR: &str = "RH_CHAOS_DIR";

fn draft() -> TaskDraft {
    TaskDraft {
        id: "chaos".into(),
        context_id: "ctx".into(),
        name: "chaos".into(),
        r#type: None,
        goal: "survive".into(),
        description: String::new(),
        acceptance_criteria: vec![],
        mode: TaskMode::LongRunning,
        participants: vec![Participant::human("u1", "sam", [Role::Reporter])],
        parent_tasks: vec![],
        reference_task_ids: vec![],
        model: rusty_harness::harness::commands::ModelSelection {
            model: "default".into(),
            effort: None,
        },
    }
}

fn event(n: u32) -> StoredEvent {
    let attempt = Attempt { iteration: 1, n: 1 };
    let mut m = a2a::Message::new(
        a2a::Role::Agent,
        vec![a2a::Part::text(format!("event {n}"))],
    );
    m.task_id = Some("chaos".into());
    StoredEvent {
        id: StoredEvent::event_id("chaos", attempt, "message", n),
        seq: 0,
        at: chrono::Utc::now(),
        attempt,
        visibility: Visibility::All,
        response: a2a::StreamResponse::Message(m),
    }
}

/// The victim: commits forever until killed. Run only by the test below.
#[test]
#[ignore = "spawned by chaos_crash_between_store_writes_leaves_a_consistent_store_on_reopen"]
fn chaos_child() {
    let dir = PathBuf::from(std::env::var(DIR_VAR).expect(DIR_VAR));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let store = FileStore::open(&dir).await.unwrap();
        let mut task = TaskRecord::new(draft(), chrono::Utc::now());
        task.transition(&Transition::Start, chrono::Utc::now())
            .unwrap();
        store.commit(TaskCommit::of(task)).await.unwrap();
        let mut n = 1;
        loop {
            let current = store.get_task(&"chaos".into()).await.unwrap().unwrap();
            let mut commit = TaskCommit::of(current).with_events([event(n)]);
            commit.blobs.push(BlobWrite {
                key: format!("turn/chaos/{n}"),
                bytes: bytes::Bytes::from(vec![b'x'; 4096]),
            });
            store.commit(commit).await.unwrap();
            n += 1;
        }
    });
}

fn journal_len(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("commits/chaos")).map_or(0, |rd| {
        rd.filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .count()
    })
}

/// Scenario: A crash between two store writes leaves a consistent store on reopen
///     Given a child process committing to a file store without pause
///     When it is killed with SIGKILL after a few dozen commits
///     Then reopening the directory replays the journal, repairs the views behind it,
///     And the record, the event log and the journal agree, and the store accepts the next commit
#[tokio::test]
async fn chaos_crash_between_store_writes_leaves_a_consistent_store_on_reopen() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("chaos-store-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "chaos_child", "--ignored", "--nocapture"])
        .env(DIR_VAR, &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the victim");
    let started = Instant::now();
    while journal_len(&dir) < 30 {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the child never got going"
        );
        if let Some(status) = child.try_wait().unwrap() {
            panic!("the child exited on its own: {status}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    child.kill().expect("SIGKILL");
    let _ = child.wait();
    let journal = journal_len(&dir);
    assert!(journal >= 30);

    let store = FileStore::open(&dir).await.expect("reopen after the crash");
    let record = store
        .get_task(&"chaos".into())
        .await
        .unwrap()
        .expect("the task survived");
    assert_eq!(
        usize::try_from(record.version).unwrap(),
        journal,
        "the record is exactly the journal's head"
    );
    let viewer = Caller::human("u1", [Role::Reporter]);
    let events = store
        .events(&"chaos".into(), 1, usize::MAX, &viewer)
        .await
        .unwrap();
    assert_eq!(
        events.len(),
        journal - 1,
        "one event per commit after the first"
    );
    assert!(
        events
            .iter()
            .enumerate()
            .all(|(i, e)| usize::try_from(e.seq).unwrap() == i + 1),
        "contiguous seqs"
    );
    for e in &events {
        assert!(
            dir.join(format!("events/chaos/{:08}.json", e.seq)).exists(),
            "every view rebuilt"
        );
    }
    let materialized: TaskRecord =
        serde_json::from_slice(&std::fs::read(dir.join("tasks/chaos.json")).unwrap()).unwrap();
    assert_eq!(materialized.version, record.version);
    assert!(
        store
            .get_blob(&format!("turn/chaos/{}", journal - 1))
            .await
            .unwrap()
            .is_some(),
        "the last committed blob is readable"
    );
    let mut leftovers = Vec::new();
    for entry in walkdir(&dir) {
        if entry.extension().is_some_and(|x| x == "tmp") {
            leftovers.push(entry);
        }
    }
    assert!(
        leftovers.is_empty(),
        "no interrupted writes remain: {leftovers:?}"
    );
    let next = store
        .commit(TaskCommit::of(record).with_events([event(u32::MAX)]))
        .await
        .unwrap();
    assert_eq!(usize::try_from(next).unwrap(), journal + 1);
    let _ = std::fs::remove_dir_all(&dir);
}

fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
        {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

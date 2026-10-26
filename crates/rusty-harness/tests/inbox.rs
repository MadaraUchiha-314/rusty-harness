//! Feature: Inbox and events
//! Requirement: docs/specs/issue-3/requirements.md#R20
//!
//! The inbox scenarios of task 22 (R20.1–6, R21.5–6, boundary 2), in process: the
//! harness's inbox over the memory store and the `none` durable mode.
#![cfg(feature = "rt-tokio")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rusty_harness::app_service::inbox::{
    DeliveryKind, EventTarget, Inbox, InboxError, InboxEvent, InboxStatus,
};
use rusty_harness::config::Config;
use rusty_harness::harness::core_loop::persistence::TaskQuery;
use rusty_harness::harness::core_loop::task::{Caller, Role, TASK_EXT};
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{ContentPart, ModelEvent, StopReason};
use rusty_harness::host::Host;
use rusty_harness::{Harness, HarnessBuilder};
use serde_json::json;

fn text(t: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta { text: t.into() },
        ModelEvent::Stop {
            reason: StopReason::EndTurn,
        },
    ]
}

async fn harness(turns: Vec<Vec<ModelEvent>>) -> (Harness, Arc<RecordedLlm>) {
    let llm = Arc::new(RecordedLlm::new("default", turns));
    let h = HarnessBuilder::new(Config::default())
        .host(Host::testing())
        .llm(llm.clone())
        .build()
        .await
        .unwrap();
    (h, llm)
}

fn message(
    id: &str,
    text: &str,
    task: Option<&str>,
    context: Option<&str>,
    long_running: bool,
) -> a2a::Message {
    let mut m = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text(text)]);
    m.message_id = id.into();
    m.task_id = task.map(str::to_owned);
    m.context_id = context.map(str::to_owned);
    if long_running {
        m.metadata = Some(HashMap::from([(
            TASK_EXT.to_owned(),
            json!({ "mode": "long-running" }),
        )]));
    }
    m
}

fn event(id: &str, target: EventTarget, kind: a2a::StreamResponse) -> InboxEvent {
    InboxEvent {
        id: id.into(),
        target,
        kind,
        status: InboxStatus::Received,
        sender: Caller::human("u1", [Role::Admin]),
        received_at: chrono::Utc::now(),
        delivery: DeliveryKind::Interrupt,
        extensions: vec![],
        traceparent: None,
    }
}

async fn only_task(h: &Harness) -> String {
    let tasks = h
        .store()
        .list_tasks(TaskQuery::everyone())
        .await
        .unwrap()
        .items;
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    tasks[0].id.clone()
}

async fn settle(h: &Harness, task: &str) {
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let running = h.agent().mailbox().is_running(&task.to_owned());
        let state = h
            .store()
            .get_task(&task.to_owned())
            .await
            .unwrap()
            .map(|r| r.state);
        if !running
            && state.is_some_and(|s| s != a2a::TaskState::Working && s != a2a::TaskState::Submitted)
        {
            return;
        }
    }
    panic!("task {task} never settled");
}

fn user_texts(llm: &RecordedLlm, i: usize) -> Vec<String> {
    llm.requests()[i]
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .filter_map(|p| match p {
            ContentPart::Text { text } if text.starts_with("[user-message]") => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Scenario: A duplicate inbox signal applies the event once
///     Given a long-running task waiting for its next turn
///     When the same follow-up event is accepted twice (a lost signal response, a retry)
///     Then the task sees one new turn and the event is applied once
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_duplicate_inbox_signal_applies_the_event_once() {
    let (h, llm) = harness(vec![text("one"), text("two"), text("three")]).await;
    let inbox: &Inbox = h.inbox().unwrap();
    inbox
        .accept(event(
            "m1",
            EventTarget::Context("ctx-1".into()),
            a2a::StreamResponse::Message(message("m1", "first", None, Some("ctx-1"), true)),
        ))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let t1 = only_task(&h).await;
    settle(&h, &t1).await;
    let follow = event(
        "m2",
        EventTarget::Task(t1.clone()),
        a2a::StreamResponse::Message(message("m2", "second", Some(&t1), Some("ctx-1"), false)),
    );
    let r1 = inbox.accept(follow.clone()).await.unwrap();
    let r2 = inbox.accept(follow).await.unwrap();
    assert_eq!(r1, r2, "the same reference both times");
    settle(&h, &t1).await;
    assert_eq!(llm.attempts(), 2, "one more model call, not two");
    let seconds = user_texts(&llm, 1)
        .into_iter()
        .filter(|t| t.contains("second"))
        .count();
    assert_eq!(seconds, 1, "the follow-up appears once");
    let record = h.store().get_task(&t1).await.unwrap().unwrap();
    assert!(
        record.applied_events.contains(&"m2".to_owned())
            || h.store().pending_inbox(&t1).await.unwrap().is_empty()
    );
    assert!(
        h.store().pending_inbox(&t1).await.unwrap().is_empty(),
        "nothing left to apply"
    );
    assert_eq!(record.state, a2a::TaskState::InputRequired);
}

/// Scenario: A follow-up to a completed task starts a new task with referenceTaskIds
///     Given a one-shot task that completed
///     When a follow-up arrives for it
///     Then the inbox refuses it as an unsupported operation
///     And the message a surface sends instead creates a new task in the same context
///     naming the finished one
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_follow_up_to_a_completed_task_starts_a_new_task_with_reference_task_ids() {
    let (h, _llm) = harness(vec![text("done"), text("again")]).await;
    let inbox = h.inbox().unwrap();
    inbox
        .accept(event(
            "m1",
            EventTarget::Context("ctx-9".into()),
            a2a::StreamResponse::Message(message("m1", "do it", None, Some("ctx-9"), false)),
        ))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let old_id = only_task(&h).await;
    settle(&h, &old_id).await;
    let old = h
        .store()
        .a2a_task(&old_id, &Caller::human("u1", []))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.status.state, a2a::TaskState::Completed);
    let follow = message("m2", "one more thing", Some(&old_id), Some("ctx-9"), false);
    let err = inbox
        .accept(event(
            "m2",
            EventTarget::Task(old_id.clone()),
            a2a::StreamResponse::Message(follow.clone()),
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(err, InboxError::Terminal(ref t) if *t == old_id),
        "{err}"
    );
    assert_eq!(
        err.to_a2a().code,
        -32004,
        "UnsupportedOperationError on the wire"
    );
    let next = Inbox::follow_up_after_terminal(&follow, &old);
    assert_eq!(next.task_id, None);
    assert_eq!(next.context_id.as_deref(), Some("ctx-9"));
    assert_eq!(
        next.reference_task_ids.as_deref(),
        Some(&[old_id.clone()][..])
    );
    let reference = inbox
        .accept(event(
            &next.message_id.clone(),
            EventTarget::Context("ctx-9".into()),
            a2a::StreamResponse::Message(next),
        ))
        .await
        .unwrap();
    let EventTarget::Context(_) = reference.target else {
        panic!("context target")
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let tasks = h
        .store()
        .list_tasks(TaskQuery::everyone())
        .await
        .unwrap()
        .items;
    let new = tasks.iter().find(|t| t.id != old_id).expect("a new task");
    assert_eq!(new.context_id, "ctx-9");
    assert_eq!(new.reference_task_ids, std::slice::from_ref(&old_id));
}

/// Abuse: an inbox event for a terminal or unknown task is rejected without creating one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn abuse_inbox_event_for_a_terminal_task_is_rejected_without_creating_one() {
    let (h, _llm) = harness(vec![text("done")]).await;
    let inbox = h.inbox().unwrap();
    inbox
        .accept(event(
            "m1",
            EventTarget::Task("t".into()),
            a2a::StreamResponse::Message(message("m1", "go", Some("t"), None, false)),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        h.store()
            .list_tasks(TaskQuery::everyone())
            .await
            .unwrap()
            .items
            .len(),
        0,
        "a message naming an unknown task creates nothing"
    );
    inbox
        .accept(event(
            "m2",
            EventTarget::Context("c".into()),
            a2a::StreamResponse::Message(message("m2", "go", Some("t"), Some("c"), false)),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        h.store()
            .list_tasks(TaskQuery::everyone())
            .await
            .unwrap()
            .items
            .len(),
        0
    );
    inbox
        .accept(event(
            "m3",
            EventTarget::Context("c".into()),
            a2a::StreamResponse::Message(message("m3", "go", None, Some("c"), false)),
        ))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let tasks = h
        .store()
        .list_tasks(TaskQuery::everyone())
        .await
        .unwrap()
        .items;
    assert_eq!(tasks.len(), 1);
    let task = tasks[0].clone();
    assert_eq!(task.state, a2a::TaskState::Completed);
    let update = a2a::TaskStatusUpdateEvent {
        task_id: task.id.clone(),
        context_id: "c".into(),
        status: a2a::TaskStatus {
            state: a2a::TaskState::Working,
            message: None,
            timestamp: None,
        },
        metadata: None,
    };
    let err = inbox
        .accept(event(
            "u1",
            EventTarget::Task(task.id.clone()),
            a2a::StreamResponse::StatusUpdate(update.clone()),
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, InboxError::Terminal(_)), "{err}");
    let err = inbox
        .accept(event(
            "u2",
            EventTarget::Task("ghost".into()),
            a2a::StreamResponse::StatusUpdate(update.clone()),
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, InboxError::NotFound(_)), "{err}");
    let err = inbox
        .accept(event(
            "u3",
            EventTarget::Context("c".into()),
            a2a::StreamResponse::StatusUpdate(a2a::TaskStatusUpdateEvent {
                task_id: String::new(),
                ..update
            }),
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, InboxError::Invalid(_)), "{err}");
    let mismatched = message("m4", "x", Some(&task.id), Some("other-ctx"), false);
    assert!(matches!(
        inbox
            .accept(event(
                "m4",
                EventTarget::Task(task.id.clone()),
                a2a::StreamResponse::Message(mismatched)
            ))
            .await
            .unwrap_err(),
        InboxError::Terminal(_)
    ));
    assert_eq!(
        h.store()
            .list_tasks(TaskQuery::everyone())
            .await
            .unwrap()
            .items
            .len(),
        1,
        "still one task"
    );
    assert_eq!(
        h.store().pending_inbox(&task.id).await.unwrap().len(),
        0,
        "nothing stored for the refused events"
    );
}

/// A follow-up accepted from inside the loop's own delivered transition: it lands after
/// the final commit and before the run returns, the window a slow store widens.
struct LateFollowUp {
    inbox: Arc<Inbox>,
    fired: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl
    rusty_harness::harness::hooks::PostHook<
        rusty_harness::harness::core_loop::task::TaskStateChange,
    > for LateFollowUp
{
    async fn post(
        &self,
        _cx: &rusty_harness::harness::hooks::HookCx,
        _input: &rusty_harness::harness::core_loop::task::StateChange,
        output: rusty_harness::harness::core_loop::task::StateChanged,
    ) -> Result<
        rusty_harness::harness::core_loop::task::StateChanged,
        rusty_harness::harness::hooks::HookError,
    > {
        if output.task.state == a2a::TaskState::InputRequired
            && !self.fired.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            let id = output.task.id.clone();
            let follow = event(
                "late-1",
                EventTarget::Task(id.clone()),
                a2a::StreamResponse::Message(message("late-1", "again", Some(&id), None, false)),
            );
            self.inbox.accept(follow).await.expect("accepted");
        }
        Ok(output)
    }
}

/// Scenario: An inbox event for a running task is applied at the next loop boundary
///     Given a long-running task whose first turn is being delivered
///     When a follow-up message is accepted after the loop's last boundary, while the run
///     is still marked running
///     Then the task runs the second turn at once, applying the message exactly once,
///     without waiting for the heartbeat to re-signal it (R20.4–5)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_accepted_in_the_runs_tail_runs_the_next_turn_once() {
    let (h, llm) = harness(vec![text("first"), text("second")]).await;
    let inbox = Arc::clone(h.inbox().unwrap());
    h.hooks()
        .register::<rusty_harness::harness::core_loop::task::TaskStateChange>(
            rusty_harness::registry::EntityId::new("test.late-follow-up"),
            rusty_harness::harness::hooks::Hook::Post(Arc::new(LateFollowUp {
                inbox: Arc::clone(&inbox),
                fired: std::sync::atomic::AtomicBool::new(false),
            })),
            rusty_harness::harness::hooks::HookOrder::default(),
        );
    inbox
        .accept(event(
            "m-1",
            EventTarget::Context("ctx-late".into()),
            a2a::StreamResponse::Message(message("m-1", "hello", None, Some("ctx-late"), true)),
        ))
        .await
        .unwrap();
    let task = only_task(&h).await;
    let mut answer = None;
    for _ in 0..300 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let record = h.store().get_task(&task).await.unwrap().unwrap();
        let text = record
            .status_message
            .as_ref()
            .and_then(|m| m.text())
            .map(str::to_owned);
        if text.as_deref() == Some("second") && !h.agent().mailbox().is_running(&task) {
            answer = text;
            break;
        }
    }
    assert_eq!(
        answer.as_deref(),
        Some("second"),
        "the late message ran the second turn"
    );
    let second_turn = user_texts(&llm, 1);
    assert_eq!(
        second_turn.iter().filter(|t| t.contains("again")).count(),
        1,
        "the follow-up appears once in the second request: {second_turn:?}"
    );
}

//! Feature: Heartbeat
//! Requirement: docs/specs/issue-3/requirements.md#R23
//!
//! Scenario: The heartbeat forwards an event whose signal failed
//!     Given a waiting task and a message stored in the inbox as received, never signalled
//!     When the heartbeat ticks
//!     Then the event is re-signalled, the loop applies it, and the liveness record is written
#![cfg(feature = "rt-tokio")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rusty_harness::HarnessBuilder;
use rusty_harness::app_service::heartbeat::Liveness;
use rusty_harness::app_service::inbox::{DeliveryKind, EventTarget, InboxEvent, InboxStatus};
use rusty_harness::config::Config;
use rusty_harness::harness::core_loop::persistence::TaskQuery;
use rusty_harness::harness::core_loop::task::{Caller, Role, TASK_EXT};
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{ModelEvent, StopReason};
use rusty_harness::host::Host;
use serde_json::json;

fn text(t: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta { text: t.into() },
        ModelEvent::Stop {
            reason: StopReason::EndTurn,
        },
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn the_heartbeat_forwards_an_event_whose_signal_failed() {
    let llm = Arc::new(RecordedLlm::new("default", vec![text("one"), text("two")]));
    let h = HarnessBuilder::new(Config::default())
        .host(Host::testing())
        .llm(llm.clone())
        .build()
        .await
        .unwrap();
    let inbox = h.inbox().unwrap();
    let mut first = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("start")]);
    first.message_id = "m1".into();
    first.context_id = Some("ctx".into());
    first.metadata = Some(HashMap::from([(
        TASK_EXT.to_owned(),
        json!({ "mode": "long-running" }),
    )]));
    let caller = Caller::human("u1", [Role::Admin]);
    inbox
        .accept(InboxEvent {
            id: "m1".into(),
            target: EventTarget::Context("ctx".into()),
            kind: a2a::StreamResponse::Message(first),
            status: InboxStatus::Received,
            sender: caller.clone(),
            received_at: chrono::Utc::now(),
            delivery: DeliveryKind::Interrupt,
            extensions: vec![],
            traceparent: None,
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let task = h
        .store()
        .list_tasks(TaskQuery::everyone())
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(task.state, a2a::TaskState::InputRequired);

    // The signal was lost: the event sits in the store as received, nothing woke the loop.
    let mut follow = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("still there?")]);
    follow.message_id = "m2".into();
    follow.task_id = Some(task.id.clone());
    h.store()
        .put_inbox(&InboxEvent {
            id: "m2".into(),
            target: EventTarget::Task(task.id.clone()),
            kind: a2a::StreamResponse::Message(follow),
            status: InboxStatus::Received,
            sender: caller,
            received_at: chrono::Utc::now(),
            delivery: DeliveryKind::Interrupt,
            extensions: vec![],
            traceparent: None,
        })
        .await
        .unwrap();
    assert_eq!(llm.attempts(), 1);

    let report = h.heartbeat().unwrap().tick().await.unwrap();
    assert_eq!(report.forwarded, ["m2"]);
    let liveness: Liveness = serde_json::from_slice(
        &h.store()
            .get_blob(&Liveness::key(&report.liveness.as_ref().unwrap().worker_id))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(liveness.open_tasks, 1);
    assert!(
        liveness.last_boundary.is_some(),
        "the loop reached a boundary before"
    );

    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if llm.attempts() == 2 && !h.agent().mailbox().is_running(&task.id) {
            break;
        }
    }
    assert_eq!(
        llm.attempts(),
        2,
        "the loop ran once more on the forwarded event"
    );
    assert!(
        h.store().pending_inbox(&task.id).await.unwrap().is_empty(),
        "applied"
    );
    let text: String = llm.requests()[1]
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .map(|p| match p {
            rusty_harness::harness::models::llm::ContentPart::Text { text } => text.clone(),
            _ => String::new(),
        })
        .collect();
    assert!(text.contains("still there?"), "{text}");
    // A second tick forwards nothing.
    assert_eq!(
        h.heartbeat().unwrap().tick().await.unwrap().forwarded.len(),
        0
    );
}

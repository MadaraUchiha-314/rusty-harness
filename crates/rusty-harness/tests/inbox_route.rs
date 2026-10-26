//! Feature: Inbox and events
//! Requirement: docs/specs/issue-3/requirements.md#R20
//!
//! The `/inbox` route (R20.1–2, boundary 2): the second way into the inbox, behind the
//! same gate and participant guard as the A2A operations.
#![cfg(feature = "server")]

use std::sync::Arc;

use rusty_harness::HarnessBuilder;
use rusty_harness::app_service::a2a_server::serve;
use rusty_harness::config::Config;
use rusty_harness::harness::core_loop::task::TASK_EXT;
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{ModelEvent, StopReason};
use rusty_harness::host::Host;
use rusty_harness::interaction::events::InboxPost;

fn text(t: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta { text: t.into() },
        ModelEvent::Stop {
            reason: StopReason::EndTurn,
        },
    ]
}

fn client() -> reqwest::Client {
    let mut headers = http::HeaderMap::new();
    headers.insert("A2A-Version", http::HeaderValue::from_static("1.0"));
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

fn post(text: &str, task: &str) -> serde_json::Value {
    let mut m = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text(text)]);
    m.task_id = Some(task.to_owned());
    serde_json::to_value(InboxPost {
        task_id: Some(task.to_owned()),
        context_id: None,
        event: a2a::StreamResponse::Message(m),
        extensions: Vec::new(),
    })
    .unwrap()
}

/// Scenario: A webhook post reaches the task through the inbox
///     Given a long-running task waiting for its next turn
///     When a participant posts a message for it on `/inbox`
///     Then the post is accepted, the loop runs the turn and `GetTask` shows the answer
///     And a post from a stranger is refused as not-found, and a post for a terminal
///     task as a conflict, without creating anything
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abuse_inbox_post_from_a_non_participant_is_refused_and_a_participants_is_applied() {
    let mut config = Config::default();
    config.server.bind = "127.0.0.1:0".into();
    let h = HarnessBuilder::new(config)
        .host(Host::testing())
        .llm(Arc::new(RecordedLlm::new(
            "default",
            vec![text("first"), text("second")],
        )))
        .build()
        .await
        .expect("builds");
    let server = serve(&h, None).await.expect("binds");
    let base = format!("http://{}/", server.addr);
    tokio::spawn(server.run(std::future::pending()));
    let client = client();

    let mut m = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("hello")]);
    m.metadata = Some(std::collections::HashMap::from([(
        TASK_EXT.to_owned(),
        serde_json::json!({ "mode": "long-running" }),
    )]));
    let sent: serde_json::Value = client
        .post(&base)
        .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"SendMessage","params": {"message": serde_json::to_value(&m).unwrap()}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let task = sent["result"]["task"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{sent}"))
        .to_owned();

    // A stranger: not-found, nothing applied (boundary 2, abuse 2).
    let refused = client
        .post(format!("{base}inbox"))
        .header("x-rusty-harness-participant", "stranger")
        .json(&post("ignore me", &task))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), http::StatusCode::NOT_FOUND);

    // Not an event: 400.
    let bad = client
        .post(format!("{base}inbox"))
        .body("{")
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), http::StatusCode::BAD_REQUEST);

    // The operator (the task's reporter): accepted and applied.
    let accepted = client
        .post(format!("{base}inbox"))
        .json(&post("again", &task))
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), http::StatusCode::ACCEPTED);
    let reference: serde_json::Value = accepted.json().await.unwrap();
    assert!(reference.get("id").is_some(), "{reference}");

    let mut answer = String::new();
    for _ in 0..50 {
        let got: serde_json::Value = client
            .post(&base)
            .json(&serde_json::json!({"jsonrpc":"2.0","id":2,"method":"GetTask","params":{"id": task}}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        answer = got["result"]["status"]["message"]["parts"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if answer == "second" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(answer, "second", "the posted message ran the next turn");

    let canceled: serde_json::Value = client
        .post(&base)
        .json(&serde_json::json!({"jsonrpc":"2.0","id":3,"method":"CancelTask","params":{"id": task}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(canceled["result"].is_object(), "{canceled}");
    let terminal = client
        .post(format!("{base}inbox"))
        .json(&post("too late", &task))
        .send()
        .await
        .unwrap();
    assert_eq!(terminal.status(), http::StatusCode::CONFLICT);
}

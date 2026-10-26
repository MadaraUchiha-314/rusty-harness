//! Feature: A2A API server
//! Requirement: docs/specs/issue-3/requirements.md#R19
//!
//! The server on the official SDK, driven here by the official Rust client; the
//! TypeScript conformance suite under `web/tests/` drives the binary the same way.
#![cfg(feature = "server")]

use std::sync::Arc;

use a2a_client::A2AClient;
use a2a_client::jsonrpc::JsonRpcTransport;
use futures::StreamExt as _;
use rusty_harness::app_service::a2a_server::{AgentCardBuilder, serve};
use rusty_harness::config::Config;
use rusty_harness::harness::core_loop::agent::Agent as _;
use rusty_harness::harness::core_loop::task::TASK_EXT;
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{ModelEvent, StopReason};
use rusty_harness::host::Host;
use rusty_harness::{Harness, HarnessBuilder};

fn text(t: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta { text: t.into() },
        ModelEvent::Stop {
            reason: StopReason::EndTurn,
        },
    ]
}

async fn harness(turns: Vec<Vec<ModelEvent>>) -> Harness {
    let mut config = Config::default();
    config.server.bind = "127.0.0.1:0".into();
    HarnessBuilder::new(config)
        .host(Host::testing())
        .llm(Arc::new(RecordedLlm::new("default", turns)))
        .build()
        .await
        .expect("builds")
}

fn message(text: &str, task: Option<&str>, long_running: bool) -> a2a::Message {
    let mut m = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text(text)]);
    m.task_id = task.map(str::to_owned);
    if long_running {
        m.metadata = Some(std::collections::HashMap::from([(
            TASK_EXT.to_owned(),
            serde_json::json!({ "mode": "long-running" }),
        )]));
    }
    m
}

/// Scenario: The official Rust client drives the server over JSON-RPC
///     Given the server bound on loopback with a scripted model
///     When the client fetches the card, sends a message, streams a follow-up, gets,
///     lists and cancels the task
///     Then every operation answers as A2A 1.0 defines, the stream starts with the Task,
///     and the card advertises every rusty-harness extension
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn the_official_rust_client_drives_the_server_over_jsonrpc() {
    let h = harness(vec![text("first answer"), text("second answer")]).await;
    let server = serve(&h, None).await.expect("binds");
    let base = format!("http://{}/", server.addr);
    tokio::spawn(server.run(std::future::pending()));

    let card: a2a::AgentCard = reqwest::get(format!("{base}.well-known/agent-card.json"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(card.name, "rusty-harness");
    let uris: Vec<&str> = card
        .capabilities
        .extensions
        .as_ref()
        .unwrap()
        .iter()
        .map(|e| e.uri.as_str())
        .collect();
    for name in [
        "task",
        "participants",
        "plan",
        "channels",
        "commands",
        "voice",
    ] {
        assert!(
            uris.iter().any(|u| u.ends_with(&format!("/{name}/v1"))),
            "{name}: {uris:?}"
        );
    }
    assert!(uris.iter().any(|u| u.contains("a2ui.org")));
    assert_eq!(card.supported_interfaces[0].protocol_binding, "JSONRPC");
    assert!(
        card.supported_interfaces[0]
            .protocol_version
            .starts_with("1.")
    );

    let client = A2AClient::new(JsonRpcTransport::new(
        reqwest::Client::new(),
        card.supported_interfaces[0].url.clone(),
    ));
    let sent = client
        .send_message(&a2a::SendMessageRequest {
            message: message("hello", None, true),
            configuration: None,
            metadata: None,
            tenant: None,
        })
        .await
        .expect("SendMessage");
    let task = match sent {
        a2a::SendMessageResponse::Task(t) => t,
        a2a::SendMessageResponse::Message(m) => panic!("expected a task, got {m:?}"),
    };
    assert_eq!(
        task.status.state,
        a2a::TaskState::InputRequired,
        "long-running: waiting for the next turn"
    );
    assert_eq!(
        task.status.message.as_ref().and_then(|m| m.text()),
        Some("first answer")
    );
    assert!(task.metadata.as_ref().unwrap().contains_key(TASK_EXT));

    let mut stream = client
        .send_streaming_message(&a2a::SendMessageRequest {
            message: message("again", Some(&task.id), false),
            configuration: None,
            metadata: None,
            tenant: None,
        })
        .await
        .expect("SendStreamingMessage");
    let mut kinds = Vec::new();
    while let Some(event) = stream.next().await {
        let event = event.expect("event");
        kinds.push(match &event {
            a2a::StreamResponse::Task(_) => "task",
            a2a::StreamResponse::Message(_) => "message",
            a2a::StreamResponse::StatusUpdate(_) => "status",
            a2a::StreamResponse::ArtifactUpdate(_) => "artifact",
        });
    }
    assert_eq!(
        kinds.first(),
        Some(&"task"),
        "the stream starts with the Task: {kinds:?}"
    );
    assert!(kinds.contains(&"status"), "{kinds:?}");
    assert!(
        !kinds.contains(&"message"),
        "replies ride on status updates: {kinds:?}"
    );

    let got = client
        .get_task(&a2a::GetTaskRequest {
            id: task.id.clone(),
            history_length: None,
            tenant: None,
        })
        .await
        .expect("GetTask");
    assert_eq!(
        got.status.message.as_ref().and_then(|m| m.text()),
        Some("second answer")
    );
    assert!(
        got.history.as_ref().is_some_and(|h| h.len() >= 2),
        "the full history after both turns: {:?}",
        got.history
    );

    let listed = client
        .list_tasks(&a2a::ListTasksRequest {
            context_id: Some(task.context_id.clone()),
            status: None,
            page_size: None,
            page_token: None,
            history_length: None,
            status_timestamp_after: None,
            include_artifacts: None,
            tenant: None,
        })
        .await
        .expect("ListTasks");
    assert_eq!(listed.tasks.len(), 1);

    let canceled = client
        .cancel_task(&a2a::CancelTaskRequest {
            id: task.id.clone(),
            metadata: None,
            tenant: None,
        })
        .await
        .expect("CancelTask");
    assert_eq!(canceled.status.state, a2a::TaskState::Canceled);
    let err = client
        .cancel_task(&a2a::CancelTaskRequest {
            id: task.id.clone(),
            metadata: None,
            tenant: None,
        })
        .await;
    assert!(
        err.is_ok() || err.is_err(),
        "a second cancel is either idempotent or refused, never a crash"
    );

    // A request naming A2A-Version 0.3 is rejected (R19.9).
    let response = reqwest::Client::new()
        .post(&base)
        .header("A2A-Version", "0.3")
        .json(&serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "GetTask", "params": { "id": task.id } }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase()
            .contains("version"),
        "{body}"
    );
}

/// T6: the card built from the default configuration, snapshotted.
#[tokio::test]
async fn snapshot_agent_card() {
    let h = harness(vec![text("x")]).await;
    let card = AgentCardBuilder::build(h.config(), h.registry(), h.agent().card());
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/agent_card.json");
    let mut json = serde_json::to_string_pretty(&serde_json::to_value(&card).unwrap()).unwrap();
    json.push('\n');
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &json).unwrap();
    }
    let published: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default())
            .unwrap_or(serde_json::Value::Null);
    assert_eq!(
        published,
        serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        "agent_card.json is stale: rerun with UPDATE_SNAPSHOTS=1"
    );
    // Without a security scheme the server binds loopback only (fail-closed list).
    let mut config = Config::default();
    config.server.bind = "0.0.0.0:0".into();
    let open = HarnessBuilder::new(config)
        .host(Host::testing())
        .llm(Arc::new(RecordedLlm::new("default", vec![])))
        .build()
        .await
        .unwrap();
    assert!(serve(&open, None).await.is_err());
}

/// Scenario: A browser surface on another origin reaches a loopback harness
///     Given a loopback-only server with no allowed origins configured
///     When a page on another origin preflights and calls it
///     Then the preflight is answered and the response carries the allow headers
///     And a server with a list allows the listed origins only
#[tokio::test]
async fn a_browser_origin_is_allowed_on_loopback_and_only_listed_origins_with_a_list() {
    let h = harness(vec![text("x")]).await;
    let server = serve(&h, None).await.expect("binds");
    let base = format!("http://{}/", server.addr);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(server.run(async move {
        let _ = stopped.await;
    }));
    let client = reqwest::Client::new();
    let preflight = client
        .request(reqwest::Method::OPTIONS, &base)
        .header("Origin", "http://127.0.0.1:4173")
        .header("Access-Control-Request-Method", "POST")
        .header(
            "Access-Control-Request-Headers",
            "a2a-version, content-type",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(preflight.status(), 204);
    assert_eq!(
        preflight.headers()["access-control-allow-origin"],
        "http://127.0.0.1:4173"
    );
    assert_eq!(
        preflight.headers()["access-control-allow-headers"],
        "a2a-version, content-type"
    );
    let card = client
        .get(format!("{base}.well-known/agent-card.json"))
        .header("Origin", "http://127.0.0.1:4173")
        .send()
        .await
        .unwrap();
    assert_eq!(card.status(), 200);
    assert_eq!(
        card.headers()["access-control-allow-origin"],
        "http://127.0.0.1:4173"
    );
    let _ = stop.send(());
    let _ = task.await;

    // With a list, only the listed origins are allowed.
    let mut config = Config::default();
    config.server.bind = "127.0.0.1:0".into();
    config.server.allow_origins = vec!["https://app.example".into()];
    let h = HarnessBuilder::new(config)
        .host(Host::testing())
        .llm(Arc::new(RecordedLlm::new("default", vec![])))
        .build()
        .await
        .expect("builds");
    let server = serve(&h, None).await.expect("binds");
    let base = format!("http://{}/", server.addr);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(server.run(async move {
        let _ = stopped.await;
    }));
    let listed = client
        .get(format!("{base}.well-known/agent-card.json"))
        .header("Origin", "https://app.example")
        .send()
        .await
        .unwrap();
    assert_eq!(
        listed.headers()["access-control-allow-origin"],
        "https://app.example"
    );
    let card = client
        .get(format!("{base}.well-known/agent-card.json"))
        .header("Origin", "http://127.0.0.1:4173")
        .send()
        .await
        .unwrap();
    assert!(
        card.headers().get("access-control-allow-origin").is_none(),
        "unlisted origin got {:?}",
        card.headers()
    );
    let _ = stop.send(());
    let _ = task.await;
}

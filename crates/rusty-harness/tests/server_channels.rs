//! Feature: Channels over the A2A server, persistence across restarts, push notifications
//! Requirement: docs/specs/issue-3/requirements.md#R18
//!
//! The task 23 scenarios: the per-subscriber projection on `GetTask` and
//! `SubscribeToTask`, the full task after a restart from the file store, and the push
//! webhook allowlist (abuse 9).
#![cfg(feature = "server")]

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use rusty_harness::app_service::a2a_server::protocol_gap::PARTICIPANT_HEADER;
use rusty_harness::app_service::a2a_server::{AllowlistedPushStore, serve};
use rusty_harness::config::Config;
use rusty_harness::harness::channels::CHANNELS_EXT;
use rusty_harness::harness::commands::ModelSelection;
use rusty_harness::harness::core_loop::persistence::TaskCommit;
use rusty_harness::harness::core_loop::task::{
    Participant, ParticipantKind, Role, TASK_EXT, TaskDraft, TaskMode, TaskRecord,
};
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{ModelEvent, StopReason};
use rusty_harness::harness::tools::ToolCall;
use rusty_harness::host::Host;
use rusty_harness::{Harness, HarnessBuilder};
use serde_json::{Value, json};

fn text(t: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta { text: t.into() },
        ModelEvent::Stop {
            reason: StopReason::EndTurn,
        },
    ]
}

async fn harness(turns: Vec<Vec<ModelEvent>>, configure: impl FnOnce(&mut Config)) -> Harness {
    let mut config = Config::default();
    config.server.bind = "127.0.0.1:0".into();
    configure(&mut config);
    HarnessBuilder::new(config)
        .host(Host::testing())
        .llm(Arc::new(RecordedLlm::new("default", turns)))
        .build()
        .await
        .expect("builds")
}

/// A running server and the handle that shuts it down.
struct Running {
    base: String,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Running {
    /// Stops the server (its heartbeat task ends with the runtime).
    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

async fn start(h: &Harness) -> Running {
    let server = serve(h, None).await.expect("binds");
    let base = format!("http://{}/", server.addr);
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(server.run(async move {
        let _ = rx.await;
    }));
    Running {
        base,
        stop: Some(tx),
    }
}

/// One JSON-RPC call, bounded so a stall names the call instead of hanging the build.
async fn rpc(base: &str, participant: &str, method: &str, params: Value) -> Value {
    let call = async {
        reqwest::Client::new()
            .post(base)
            .header("A2A-Version", "1.0")
            .header(PARTICIPANT_HEADER, participant)
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    };
    tokio::time::timeout(std::time::Duration::from_secs(60), call)
        .await
        .unwrap_or_else(|_| panic!("{method} as {participant} did not answer within 60 s"))
}

fn message(text: &str, task: Option<&str>) -> Value {
    let mut m = json!({ "messageId": uuid::Uuid::new_v4().to_string(), "role": "ROLE_USER", "parts": [{ "text": text }], "metadata": { TASK_EXT: { "mode": "long-running" } } });
    if let Some(t) = task {
        m["taskId"] = json!(t);
    }
    m
}

/// Scenario: A task watcher outside a channel sees the state change but not the question
///     Given a task with alice (admin) and bob (watcher) and a question asked of alice
///     When bob reads the task and subscribes to it
///     Then bob sees input-required and the sub-state but no question parts
///     And alice sees the question
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_watcher_outside_a_channel_sees_the_state_change_but_not_the_question() {
    let ask = vec![
        ModelEvent::ToolCall {
            call: ToolCall {
                id: "c1".into(),
                name: "participant.ask".into(),
                arguments: json!({ "to": ["alice"], "question": "Deploy to prod?" }),
            },
        },
        ModelEvent::Stop {
            reason: StopReason::ToolUse,
        },
    ];
    let h = harness(vec![ask, text("Deploying.")], |_| {}).await;
    // The task exists with two people before the first message (the server creates
    // tasks with their creator only; participants are managed by the harness).
    let record = TaskRecord::new(
        TaskDraft {
            id: "shared".into(),
            context_id: "ctx".into(),
            name: "shared".into(),
            r#type: None,
            goal: "ship".into(),
            description: String::new(),
            acceptance_criteria: vec![],
            mode: TaskMode::LongRunning,
            participants: vec![
                Participant::human("alice", "Alice", [Role::Reporter, Role::Admin]),
                Participant::human("bob", "Bob", [Role::Watcher]),
                Participant {
                    id: "rusty-harness".into(),
                    display: "rusty-harness".into(),
                    kind: ParticipantKind::System,
                    roles: BTreeSet::from([Role::Assignee]),
                },
            ],
            parent_tasks: vec![],
            reference_task_ids: vec![],
            model: ModelSelection {
                model: "default".into(),
                effort: None,
            },
        },
        chrono::Utc::now(),
    );
    h.store().commit(TaskCommit::of(record)).await.unwrap();
    let mut running = start(&h).await;
    let base = running.base.clone();
    let sent = rpc(
        &base,
        "alice",
        "SendMessage",
        json!({ "message": message("ship it", Some("shared")) }),
    )
    .await;
    assert_eq!(
        sent["result"]["task"]["status"]["state"], "TASK_STATE_INPUT_REQUIRED",
        "{sent}"
    );
    let question = &sent["result"]["task"]["status"]["message"];
    assert_eq!(
        question["parts"][0]["text"], "Deploy to prod?",
        "alice sees the question: {sent}"
    );

    let bob_view = rpc(&base, "bob", "GetTask", json!({ "id": "shared" })).await;
    let status = &bob_view["result"]["status"];
    assert_eq!(status["state"], "TASK_STATE_INPUT_REQUIRED", "{bob_view}");
    assert!(
        status["message"]["parts"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "no question for bob: {bob_view}"
    );
    assert_eq!(
        status["message"]["metadata"][CHANNELS_EXT]["redacted"], true,
        "{bob_view}"
    );
    assert_eq!(
        status["message"]["metadata"][TASK_EXT]["sub_state"]["kind"], "awaiting-participant",
        "{bob_view}"
    );
    let alice_view = rpc(&base, "alice", "GetTask", json!({ "id": "shared" })).await;
    assert_eq!(
        alice_view["result"]["status"]["message"]["parts"][0]["text"],
        "Deploy to prod?"
    );

    // Bob's subscription replays the state change without the question.
    let sse = reqwest::Client::new()
        .post(&base)
        .header("A2A-Version", "1.0")
        .header(PARTICIPANT_HEADER, "bob")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "SubscribeToTask", "params": { "id": "shared" } }))
        .send()
        .await
        .unwrap();
    let body = tokio::time::timeout(Duration::from_secs(5), sse.text())
        .await
        .expect("replay")
        .unwrap();
    assert!(!body.contains("Deploy to prod?"), "{body}");
    assert!(body.contains("TASK_STATE_INPUT_REQUIRED"), "{body}");
    running.stop();
}

/// Scenario: `GetTask` after a restart returns the full A2A task
///     Given a task completed against the file store
///     When a new harness opens the same store
///     Then `GetTask` returns the task with its status, history and extension metadata
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_task_after_a_restart_returns_the_full_a2a_task() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("restart-{}", uuid::Uuid::new_v4()));
    let file = |c: &mut Config| {
        c.persistence.store = "file".into();
        c.persistence.path = Some(dir.clone());
    };
    let h = harness(vec![text("first"), text("second")], file).await;
    let mut running = start(&h).await;
    let base = running.base.clone();
    let sent = rpc(
        &base,
        "alice",
        "SendMessage",
        json!({ "message": message("hello", None) }),
    )
    .await;
    let task = sent["result"]["task"]["id"].as_str().unwrap().to_owned();
    rpc(
        &base,
        "alice",
        "SendMessage",
        json!({ "message": message("again", Some(&task)) }),
    )
    .await;
    // The first process ends: its server stops before the second one opens the store.
    running.stop();
    drop(h);

    let h2 = harness(vec![], file).await;
    let mut running2 = start(&h2).await;
    let base2 = running2.base.clone();
    let got = rpc(&base2, "alice", "GetTask", json!({ "id": task })).await;
    assert_eq!(got["result"]["id"], task, "{got}");
    assert_eq!(
        got["result"]["status"]["state"],
        "TASK_STATE_INPUT_REQUIRED"
    );
    assert_eq!(
        got["result"]["status"]["message"]["parts"][0]["text"],
        "second"
    );
    assert!(
        got["result"]["history"]
            .as_array()
            .is_some_and(|h| h.len() >= 2),
        "{got}"
    );
    assert_eq!(got["result"]["metadata"][TASK_EXT]["mode"], "long-running");
    let listed = rpc(&base2, "alice", "ListTasks", json!({})).await;
    assert_eq!(listed["result"]["tasks"].as_array().map_or(0, Vec::len), 1);
    running2.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Abuse case 9: A push config naming 169.254.169.254 is rejected.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn abuse_push_config_naming_a_link_local_address_is_rejected() {
    let store = AllowlistedPushStore::new(vec!["hooks.example.com".into()]);
    assert!(
        store
            .check("http://169.254.169.254/latest/meta-data")
            .is_err(),
        "the metadata service"
    );
    assert!(
        store.check("https://10.0.0.8/hook").is_err(),
        "a private address"
    );
    assert!(
        store.check("https://evil.example/hook").is_err(),
        "off the allowlist"
    );
    assert!(
        store.check("ftp://hooks.example.com/hook").is_err(),
        "wrong scheme"
    );
    assert!(store.check("https://HOOKS.example.com/hook").is_ok());
    assert!(
        AllowlistedPushStore::new(vec![])
            .check("https://hooks.example.com/hook")
            .is_err(),
        "an empty allowlist refuses everything"
    );

    let h = harness(vec![text("ok")], |c| {
        c.server.push.allow_hosts = vec!["hooks.example.com".into()];
    })
    .await;
    let mut running = start(&h).await;
    let base = running.base.clone();
    let card: Value = reqwest::get(format!("{base}.well-known/agent-card.json"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(card["capabilities"]["pushNotifications"], true);
    let sent = rpc(
        &base,
        "alice",
        "SendMessage",
        json!({ "message": message("hello", None) }),
    )
    .await;
    let task = sent["result"]["task"]["id"].as_str().unwrap().to_owned();
    let bad = rpc(
        &base,
        "alice",
        "CreateTaskPushNotificationConfig",
        json!({ "taskId": task, "url": "http://169.254.169.254/hook" }),
    )
    .await;
    assert!(bad["error"].is_object(), "{bad}");
    let off = rpc(
        &base,
        "alice",
        "CreateTaskPushNotificationConfig",
        json!({ "taskId": task, "url": "https://evil.example/hook" }),
    )
    .await;
    assert!(
        off["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("allow_hosts"),
        "{off}"
    );
    let ok = rpc(
        &base,
        "alice",
        "CreateTaskPushNotificationConfig",
        json!({ "taskId": task, "url": "https://hooks.example.com/hook" }),
    )
    .await;
    assert_eq!(
        ok["result"]["url"], "https://hooks.example.com/hook",
        "{ok}"
    );
    let listed = rpc(
        &base,
        "alice",
        "ListTaskPushNotificationConfigs",
        json!({ "taskId": task }),
    )
    .await;
    assert_eq!(
        listed["result"]["configs"].as_array().map_or(0, Vec::len),
        1,
        "{listed}"
    );
    let bob = rpc(
        &base,
        "bob",
        "ListTaskPushNotificationConfigs",
        json!({ "taskId": task }),
    )
    .await;
    assert_eq!(
        bob["error"]["code"], -32001,
        "a non-participant cannot read push configs: {bob}"
    );
    let _: HashMap<String, String> = HashMap::new();
    running.stop();
}

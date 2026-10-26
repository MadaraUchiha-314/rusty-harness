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

use futures::StreamExt as _;

use rusty_harness::app_service::a2a_server::protocol_gap::PARTICIPANT_HEADER;
use rusty_harness::app_service::a2a_server::{AllowlistedPushStore, serve};
use rusty_harness::config::Config;
use rusty_harness::harness::channels::CHANNELS_EXT;
use rusty_harness::harness::commands::ModelSelection;
use rusty_harness::harness::core_loop::persistence::TaskCommit;
use rusty_harness::harness::core_loop::task::{
    Caller, Participant, ParticipantKind, Role, TASK_EXT, TaskDraft, TaskMode, TaskRecord,
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
///     And alice sees the question, through the server and through the SDK alike
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one scenario: the server, the stream and the SDK
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

    // The SDK's getTask applies the same rule as the server's.
    let as_bob = h
        .get_task("shared", Some(Caller::human("bob", [Role::Watcher])))
        .await
        .expect("bob reads the task");
    let bob_message = as_bob.status.message.as_ref().expect("a status message");
    assert!(
        bob_message.parts.is_empty(),
        "no question for bob through the SDK: {bob_message:?}"
    );
    let as_alice = h
        .get_task("shared", Some(Caller::human("alice", [Role::Admin])))
        .await
        .expect("alice reads the task");
    assert_eq!(
        as_alice.status.message.as_ref().and_then(|m| m.text()),
        Some("Deploy to prod?"),
        "alice sees the question through the SDK"
    );
    running.stop();
}

/// Scenario: `GetTask` after a restart returns the full A2A task
///     Given a task completed against the file store
///     When a new harness opens the same store
///     Then `GetTask` returns the task with its status, history and extension metadata
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_task_after_a_restart_returns_the_full_a2a_task() {
    // The loop's spans and logs on stderr: nextest keeps them when the test fails, so a
    // stall on CI shows where it stopped.
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug,hyper=info,h2=info,tower=info,reqwest=info")
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
        .with_writer(std::io::stderr)
        .try_init();
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

/// Scenario: A command on the server runs with the sender's roles on the task
///     Given a shared task where alice is an admin and bob a watcher
///     When each sends `/model default` into the task
///     Then the role guard denies bob and lets alice
///     And bob's `/model default` outside any task is an ordinary message, not a command
///     (R10.4, boundary 4)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_on_the_server_runs_with_the_senders_roles_on_the_task() {
    use rusty_harness::harness::commands::{COMMANDS_EXT, CommandsMeta};
    let h = harness(vec![text("an ordinary answer")], |_| {}).await;
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
    let outcome = |reply: &Value| -> Option<String> {
        let meta: CommandsMeta = rusty_harness::harness::core_loop::task::decode_extension(
            &reply["result"]["message"]["metadata"][COMMANDS_EXT],
        )
        .ok()?;
        meta.result.map(|r| r.outcome)
    };

    let said = |reply: &Value| {
        reply["result"]["message"]["parts"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    let send = |who: &'static str, text: &'static str, task: Option<&'static str>| {
        let base = base.clone();
        async move {
            rpc(
                &base,
                who,
                "SendMessage",
                json!({ "message": message(text, task) }),
            )
            .await
        }
    };

    // The role guard answers before the command runs: the denial, not "no such model".
    let bob = send("bob", "/model default", Some("shared")).await;
    assert_eq!(outcome(&bob).as_deref(), Some("failed"), "{bob}");
    assert!(
        said(&bob).contains("Watchers may not run"),
        "a watcher may not switch models: {bob}"
    );

    let alice = send("alice", "/model default", Some("shared")).await;
    assert_eq!(
        outcome(&alice).as_deref(),
        Some("done"),
        "an admin of the task may: {alice}"
    );

    // Outside any task an authenticated sender is not the server's operator: the text
    // goes to the agent as a message and opens a task.
    let outside = send("bob", "/model default", None).await;
    assert!(
        outside["result"]["task"].is_object(),
        "an authenticated sender's /text outside a task is a message to the agent: {outside}"
    );
    running.stop();
}

/// A long-running task `id` with alice (admin, reporter) and bob (watcher) on it.
fn shared_task(id: &str) -> TaskRecord {
    TaskRecord::new(
        TaskDraft {
            id: id.into(),
            context_id: format!("ctx-{id}"),
            name: id.into(),
            r#type: None,
            goal: "ship".into(),
            description: String::new(),
            acceptance_criteria: vec![],
            mode: TaskMode::LongRunning,
            participants: vec![
                Participant::human("alice", "Alice", [Role::Reporter, Role::Admin]),
                Participant::human("bob", "Bob", [Role::Watcher]),
            ],
            parent_tasks: vec![],
            reference_task_ids: vec![],
            model: ModelSelection {
                model: "default".into(),
                effort: None,
            },
        },
        chrono::Utc::now(),
    )
}

/// Scenario: Only a task's participants open it through the SDK, and a cancel keeps the
/// channel's question for its members
///     Given two shared tasks, each waiting on a question asked of alice only
///     When mallory, on neither task, reads, follows, posts into or cancels one through the SDK
///     Then each call answers "task not found"
///     And when bob cancels one through the SDK and the other through the server
///     Then both cancelled tasks come back without the question (R18.5, abuse 2)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_participants_open_a_task_and_a_cancel_keeps_the_question_private() {
    let ask = || {
        vec![
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
        ]
    };
    let h = harness(vec![ask(), ask()], |_| {}).await;
    for id in ["one", "two"] {
        h.store()
            .commit(TaskCommit::of(shared_task(id)))
            .await
            .unwrap();
    }
    let mut running = start(&h).await;
    let base = running.base.clone();
    for id in ["one", "two"] {
        let sent = rpc(
            &base,
            "alice",
            "SendMessage",
            json!({ "message": message("ship it", Some(id)) }),
        )
        .await;
        assert_eq!(
            sent["result"]["task"]["status"]["state"], "TASK_STATE_INPUT_REQUIRED",
            "{sent}"
        );
    }

    let mallory = || Some(Caller::human("mallory", [Role::Admin]));
    assert!(
        h.get_task("one", mallory()).await.is_err(),
        "mallory reads nothing"
    );
    assert!(
        h.subscribe("one", mallory()).await.is_err(),
        "mallory follows nothing"
    );
    assert!(
        h.cancel_task("one", mallory()).await.is_err(),
        "mallory cancels nothing"
    );
    let mut note = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("ship it now")]);
    note.task_id = Some("one".into());
    let posted: Vec<_> = h
        .send_message(
            a2a::SendMessageRequest {
                message: note,
                configuration: None,
                metadata: None,
                tenant: None,
            },
            mallory(),
        )
        .collect()
        .await;
    assert!(
        matches!(posted.as_slice(), [Err(_)]),
        "mallory posts nothing into the task: {posted:?}"
    );
    assert_eq!(
        h.get_task("one", Some(Caller::human("alice", [Role::Admin])))
            .await
            .expect("still there")
            .status
            .state,
        a2a::TaskState::InputRequired,
        "mallory's cancel changed nothing"
    );

    let by_sdk = h
        .cancel_task("one", Some(Caller::human("bob", [Role::Watcher])))
        .await
        .expect("bob may cancel");
    assert!(
        by_sdk
            .status
            .message
            .as_ref()
            .is_none_or(|m| m.parts.is_empty()),
        "no question in the SDK's cancel answer: {by_sdk:?}"
    );
    let by_server = rpc(&base, "bob", "CancelTask", json!({ "id": "two" })).await;
    assert!(
        !by_server.to_string().contains("Deploy to prod?"),
        "no question in the server's cancel answer: {by_server}"
    );
    running.stop();
}

//! Feature: A2A API server, security
//! Requirement: docs/specs/issue-3/requirements.md#R19
//!
//! The abuse scenarios of task 21 (design.md § Security design, boundaries 1 and 2,
//! abuse cases 1, 2 and 8) and the extension negotiation scenario (R19.3), driven over
//! raw JSON-RPC so every header is under the test's control.
#![cfg(feature = "server")]

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use rusty_harness::app_service::a2a_server::protocol_gap::{PARTICIPANT_HEADER, error_code};
use rusty_harness::app_service::a2a_server::serve;
use rusty_harness::config::{Config, SecuritySchemeConfig};
use rusty_harness::harness::core_loop::persistence::TaskQuery;
use rusty_harness::harness::core_loop::task::{Caller, TASK_EXT};
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{ModelEvent, StopReason};
use rusty_harness::harness::tools::{
    Idempotency, Tool, ToolCall, ToolCx, ToolDescriptor, ToolError, ToolOutput,
};
use rusty_harness::host::testing::{
    FixedClock, MapSecrets, MemFs, MockHttp, RecordingSink, SeededRng,
};
use rusty_harness::host::{Host, SecretRef};
use rusty_harness::registry::{Entity, EntityId, EntityKind};
use rusty_harness::{Harness, HarnessBuilder};
use serde_json::{Value, json};

const TOKEN: &str = "s3cret-token";

fn text(t: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta { text: t.into() },
        ModelEvent::Stop {
            reason: StopReason::EndTurn,
        },
    ]
}

/// A tool that sleeps, so a task stays `working` while streams are open.
struct Sleep;
impl Entity for Sleep {
    fn kind(&self) -> EntityKind {
        EntityKind::Tool
    }
    fn entity_id(&self) -> &EntityId {
        static ID: std::sync::OnceLock<EntityId> = std::sync::OnceLock::new();
        ID.get_or_init(|| EntityId::new("sleep"))
    }
}
#[async_trait::async_trait]
impl Tool for Sleep {
    fn descriptor(&self) -> &ToolDescriptor {
        static D: std::sync::OnceLock<ToolDescriptor> = std::sync::OnceLock::new();
        D.get_or_init(|| ToolDescriptor {
            name: "sleep".into(),
            description: "waits".into(),
            parameters: json!({ "type": "object" }),
            idempotency: Idempotency::Idempotent,
        })
    }
    async fn call(&self, _cx: &ToolCx, _input: Value) -> Result<ToolOutput, ToolError> {
        tokio::time::sleep(Duration::from_secs(3)).await;
        Ok(ToolOutput::Value {
            value: json!("slept"),
        })
    }
}

fn host() -> Host {
    Host::builder()
        .http(Arc::new(MockHttp::always(
            http::StatusCode::NOT_FOUND,
            bytes::Bytes::new(),
        )))
        .clock(Arc::new(FixedClock::default()))
        .rng(Arc::new(SeededRng::new(7)))
        .secrets(Arc::new(MapSecrets::new([(
            SecretRef::from_str("env:RH_TEST_TOKEN").unwrap(),
            TOKEN.to_owned(),
        )])))
        .fs(Arc::new(MemFs::default()))
        .log_sink(Arc::new(RecordingSink::default()))
        .build()
}

async fn harness(
    turns: Vec<Vec<ModelEvent>>,
    secured: bool,
    limits: Option<(u32, u32)>,
) -> Harness {
    let mut config = Config::default();
    config.server.bind = "127.0.0.1:0".into();
    if secured {
        config.server.security = Some(SecuritySchemeConfig::Bearer {
            token: SecretRef::from_str("env:RH_TEST_TOKEN").unwrap(),
        });
    }
    if let Some((rpm, streams)) = limits {
        config.server.limits.requests_per_minute = rpm;
        config.server.limits.streams_per_caller = streams;
    }
    HarnessBuilder::new(config)
        .host(host())
        .llm(Arc::new(RecordedLlm::new("default", turns)))
        .tool(Arc::new(Sleep))
        .build()
        .await
        .expect("builds")
}

async fn start(h: &Harness) -> String {
    let server = serve(h, None).await.expect("binds");
    let base = format!("http://{}/", server.addr);
    tokio::spawn(server.run(std::future::pending()));
    base
}

fn message(text: &str, task: Option<&str>) -> Value {
    let mut m = json!({ "messageId": uuid::Uuid::new_v4().to_string(), "role": "ROLE_USER", "parts": [{ "text": text }], "metadata": { TASK_EXT: { "mode": "long-running" } } });
    if let Some(t) = task {
        m["taskId"] = json!(t);
    }
    m
}

async fn rpc(base: &str, headers: &[(&str, &str)], method: &str, params: Value) -> (u16, Value) {
    let mut req = reqwest::Client::new()
        .post(base)
        .header("A2A-Version", "1.0")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }));
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = req.send().await.unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

fn error_code_of(body: &Value) -> Option<i64> {
    body["error"]["code"].as_i64()
}

/// Abuse case 1: Unauthenticated `SendMessage` is rejected before any task is created.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abuse_unauthenticated_send_message_is_rejected_before_any_task_is_created() {
    let h = harness(vec![text("hi")], true, None).await;
    let base = start(&h).await;
    let (status, body) = rpc(
        &base,
        &[],
        "SendMessage",
        json!({ "message": message("hello", None) }),
    )
    .await;
    assert_eq!(status, 401, "{body}");
    assert_eq!(
        error_code_of(&body),
        Some(i64::from(error_code::AUTHENTICATION_REQUIRED))
    );
    let (status, body) = rpc(
        &base,
        &[("Authorization", "Bearer wrong")],
        "SendMessage",
        json!({ "message": message("hello", None) }),
    )
    .await;
    assert_eq!(status, 401, "{body}");
    assert_eq!(
        h.store()
            .list_tasks(TaskQuery::everyone())
            .await
            .unwrap()
            .items
            .len(),
        0,
        "no task was created"
    );
    // The card stays public; the right token gets through.
    let card: Value = reqwest::get(format!("{base}.well-known/agent-card.json"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(card["securitySchemes"]["bearer"].is_object(), "{card}");
    let (status, body) = rpc(
        &base,
        &[("Authorization", &format!("Bearer {TOKEN}"))],
        "SendMessage",
        json!({ "message": message("hello", None) }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["result"]["task"]["id"].is_string(), "{body}");
}

/// Abuse case 2: A non-participant gets `TaskNotFound` for `GetTask`, `SubscribeToTask` and
/// `CancelTask`, and an empty page from `ListTasks`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abuse_non_participant_gets_task_not_found_and_empty_list_tasks() {
    let h = harness(vec![text("one"), text("two")], false, None).await;
    let base = start(&h).await;
    let alice = [(PARTICIPANT_HEADER, "alice")];
    let bob = [(PARTICIPANT_HEADER, "bob")];
    let (_, body) = rpc(
        &base,
        &alice,
        "SendMessage",
        json!({ "message": message("alice's task", None) }),
    )
    .await;
    let task = body["result"]["task"]["id"]
        .as_str()
        .expect("task")
        .to_owned();
    for method in ["GetTask", "CancelTask", "SubscribeToTask"] {
        let (status, body) = rpc(&base, &bob, method, json!({ "id": task })).await;
        assert_eq!(
            error_code_of(&body),
            Some(-32001),
            "{method} for bob: {status} {body}"
        );
    }
    let (_, body) = rpc(
        &base,
        &bob,
        "SendMessage",
        json!({ "message": message("mine now", Some(&task)) }),
    )
    .await;
    assert_eq!(
        error_code_of(&body),
        Some(-32001),
        "a follow-up by a non-participant: {body}"
    );
    let (_, body) = rpc(&base, &bob, "ListTasks", json!({})).await;
    assert_eq!(
        body["result"]["tasks"].as_array().map_or(0, Vec::len),
        0,
        "an empty page (ProtoJSON omits an empty list): {body}"
    );
    let (_, body) = rpc(&base, &alice, "ListTasks", json!({})).await;
    assert_eq!(
        body["result"]["tasks"].as_array().map(Vec::len),
        Some(1),
        "{body}"
    );
    let (_, body) = rpc(&base, &alice, "GetTask", json!({ "id": task })).await;
    assert_eq!(body["result"]["id"], task);
    let record = h.store().get_task(&task).await.unwrap().unwrap();
    assert!(Caller::human("alice", []).sees(&record) && !Caller::human("bob", []).sees(&record));
}

/// Abuse case 8: The 9th stream of one caller is rejected and the other task keeps
/// streaming.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abuse_ninth_stream_of_one_caller_is_rejected_and_the_other_task_keeps_streaming() {
    let tool_turn = vec![
        ModelEvent::ToolCall {
            call: ToolCall {
                id: "c1".into(),
                name: "sleep".into(),
                arguments: json!({}),
            },
        },
        ModelEvent::Stop {
            reason: StopReason::ToolUse,
        },
    ];
    let h = harness(
        vec![tool_turn.clone(), text("a done"), tool_turn, text("b done")],
        false,
        Some((1000, 8)),
    )
    .await;
    let base = start(&h).await;
    let client = reqwest::Client::new();
    let open = |participant: &'static str, body: Value| {
        let client = client.clone();
        let base = base.clone();
        async move {
            client
                .post(&base)
                .header("A2A-Version", "1.0")
                .header(PARTICIPANT_HEADER, participant)
                .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "SendStreamingMessage", "params": body }))
                .send()
                .await
                .unwrap()
        }
    };
    // Alice: one streaming send (held open by the sleeping tool), then seven subscriptions.
    let first = open("alice", json!({ "message": message("work", None) })).await;
    assert!(
        first
            .headers()
            .get("content-type")
            .is_some_and(|v| v.to_str().unwrap().contains("event-stream"))
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    let alice_task = h
        .store()
        .list_tasks(TaskQuery::everyone())
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|t| t.participants.iter().any(|p| p.id == "alice"))
        .expect("alice's task")
        .id;
    let mut held = vec![first];
    for _ in 0..7 {
        let res = client.post(&base).header("A2A-Version", "1.0").header(PARTICIPANT_HEADER, "alice").json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "SubscribeToTask", "params": { "id": alice_task } })).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200);
        held.push(res);
    }
    let ninth = client.post(&base).header("A2A-Version", "1.0").header(PARTICIPANT_HEADER, "alice").json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "SubscribeToTask", "params": { "id": alice_task } })).send().await.unwrap();
    let body: Value = ninth.json().await.unwrap();
    assert_eq!(
        error_code_of(&body),
        Some(i64::from(error_code::TOO_MANY_STREAMS)),
        "{body}"
    );
    // Bob's task streams regardless.
    let bob = open("bob", json!({ "message": message("other work", None) })).await;
    assert_eq!(bob.status().as_u16(), 200);
    let chunk = tokio::time::timeout(Duration::from_secs(5), bob.text())
        .await
        .expect("bob keeps streaming")
        .unwrap();
    assert!(
        chunk.contains("\"task\"") || chunk.contains("statusUpdate"),
        "{chunk}"
    );
    drop(held);
}

/// Scenario: An extension not advertised in the card is not activated
///     Given a request naming an advertised extension and an unknown one
///     When it is served
///     Then the echoed `A2A-Extensions` header lists the advertised one only
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_extension_not_advertised_in_the_card_is_not_activated() {
    let h = harness(vec![text("ok")], false, Some((2, 8))).await;
    let base = start(&h).await;
    let res = reqwest::Client::new()
        .post(&base)
        .header("A2A-Version", "1.0")
        .header("A2A-Extensions", format!("https://example.com/not-advertised/v1, {TASK_EXT}"))
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "SendMessage", "params": { "message": message("hello", None) } }))
        .send()
        .await
        .unwrap();
    let echoed = res
        .headers()
        .get("a2a-extensions")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert_eq!(echoed, TASK_EXT, "only the advertised extension is active");
    let body: Value = res.json().await.unwrap();
    assert!(body["result"]["task"]["id"].is_string(), "{body}");
    // The per-caller token bucket: two per minute here, the third is refused.
    let (status, _) = rpc(&base, &[], "ListTasks", json!({})).await;
    assert_eq!(status, 200);
    let (status, body) = rpc(&base, &[], "ListTasks", json!({})).await;
    assert_eq!(status, 429, "{body}");
    assert_eq!(
        error_code_of(&body),
        Some(i64::from(error_code::RATE_LIMITED))
    );
    let _: HashMap<String, String> = HashMap::new();
}

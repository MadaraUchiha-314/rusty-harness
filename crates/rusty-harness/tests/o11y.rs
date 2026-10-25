//! Feature: Observability
//! Requirement: docs/specs/issue-3/requirements.md#R24
//!
//! The OTLP collector is a stub in the test: an HTTP endpoint decoding the protobuf
//! export the harness sends, so the assertion is on what a real backend would receive.
#![cfg(feature = "server")]

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message as _;
use rusty_harness::HarnessBuilder;
use rusty_harness::app_service::a2a_server::serve;
use rusty_harness::app_service::o11y::{self, Scope};
use rusty_harness::config::Config;
use rusty_harness::harness::core_loop::task::TASK_EXT;
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{ModelEvent, StopReason};
use rusty_harness::host::Host;

/// One exported span, flattened.
#[derive(Debug, Clone)]
struct Exported {
    trace_id: String,
    attributes: BTreeMap<String, String>,
}

type Seen = Arc<Mutex<Vec<Exported>>>;

/// The caller's trace id.
const TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

async fn ingest(State(seen): State<Seen>, body: axum::body::Bytes) -> axum::response::Response {
    let request = ExportTraceServiceRequest::decode(body).expect("an OTLP protobuf export");
    let mut spans = Vec::new();
    for resource in request.resource_spans {
        for scope in resource.scope_spans {
            for span in scope.spans {
                let attributes = span
                    .attributes
                    .iter()
                    .map(|kv| {
                        let value = kv
                            .value
                            .as_ref()
                            .and_then(|v| v.value.as_ref())
                            .map(|v| match v {
                                opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s) => s.clone(),
                                other => format!("{other:?}"),
                            })
                            .unwrap_or_default();
                        (kv.key.clone(), value)
                    })
                    .collect();
                spans.push(Exported {
                    trace_id: hex(&span.trace_id),
                    attributes,
                });
            }
        }
    }
    seen.lock().unwrap().extend(spans);
    axum::response::IntoResponse::into_response((
        http::StatusCode::OK,
        [("content-type", "application/x-protobuf")],
        Vec::<u8>::new(),
    ))
}

async fn collector() -> (SocketAddr, Seen) {
    let seen: Seen = Arc::default();
    let app = axum::Router::new()
        .route("/v1/traces", axum::routing::post(ingest))
        .with_state(Arc::clone(&seen));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, seen)
}

/// A client speaking A2A 1.0 (the server rejects requests naming no version, R19.9).
fn a2a_client() -> reqwest::Client {
    let mut headers = http::HeaderMap::new();
    headers.insert("A2A-Version", http::HeaderValue::from_static("1.0"));
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

fn text(t: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta { text: t.into() },
        ModelEvent::Stop {
            reason: StopReason::EndTurn,
        },
    ]
}

fn user_message(text: &str, task: Option<&str>, long_running: bool) -> serde_json::Value {
    let mut m = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text(text)]);
    m.task_id = task.map(str::to_owned);
    if long_running {
        m.metadata = Some(std::collections::HashMap::from([(
            TASK_EXT.to_owned(),
            serde_json::json!({ "mode": "long-running" }),
        )]));
    }
    serde_json::to_value(a2a::SendMessageRequest {
        message: m,
        configuration: None,
        metadata: None,
        tenant: None,
    })
    .unwrap()
}

/// Scenario: Every operation of a task appears in one trace
///     Given the server exporting to an OTLP collector and a caller with a traceparent
///     When the caller starts a long-running task and later sends a second turn without one
///     Then the request, the inbox, the task's creation and both runs of the loop export
///     spans carrying the task id under the caller's trace id, and nothing else does
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn every_operation_of_a_task_appears_in_one_trace() {
    let (collector_addr, seen) = collector().await;
    let mut config = Config::default();
    config.server.bind = "127.0.0.1:0".into();
    config.o11y.otlp_endpoint = Some(format!("http://{collector_addr}/").parse().unwrap());
    let host = Host::testing();
    let guard = o11y::init(&config.o11y, &host, Scope::Global).expect("subscriber");
    let h = HarnessBuilder::new(config)
        .host(host)
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

    let traceparent = format!("00-{TRACE}-00f067aa0ba902b7-01");
    let client = a2a_client();
    let first: serde_json::Value = client
        .post(&base)
        .header("traceparent", &traceparent)
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "SendMessage",
            "params": user_message("hello", None, true)
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let task_id = first["result"]["task"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a task: {first}"))
        .to_owned();
    // The second turn carries no traceparent: the task's trace is persisted, not
    // only propagated.
    let second: serde_json::Value = client
        .post(&base)
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "SendMessage",
            "params": user_message("again", Some(&task_id), false)
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(second["result"]["task"]["id"], task_id, "{second}");

    let mut spans = Vec::new();
    for _ in 0..50 {
        guard.flush();
        spans = seen.lock().unwrap().clone();
        let runs = spans
            .iter()
            .filter(|s| s.attributes.get("point").map(String::as_str) == Some("task.run"))
            .count();
        if runs >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let ours: Vec<&Exported> = spans
        .iter()
        .filter(|s| s.attributes.get("task").map(String::as_str) == Some(task_id.as_str()))
        .collect();
    let points: BTreeSet<&str> = ours
        .iter()
        .filter_map(|s| s.attributes.get("point").map(String::as_str))
        .collect();
    for point in [
        "inbox.accept",
        "task.create",
        "task.run",
        "context.build",
        "model.call",
        "response.parse",
        "task.state_change",
    ] {
        assert!(points.contains(point), "{point} missing from {points:?}");
    }
    assert_eq!(
        ours.iter()
            .filter(|s| s.attributes.get("point").map(String::as_str) == Some("task.run"))
            .count(),
        2,
        "both turns"
    );
    let traces: BTreeSet<&str> = ours.iter().map(|s| s.trace_id.as_str()).collect();
    assert_eq!(
        traces,
        BTreeSet::from([TRACE]),
        "one trace per task, the caller's: {ours:#?}"
    );
    assert!(
        ours.iter().all(|s| s
            .attributes
            .get("traceparent")
            .is_some_and(|t| t.contains(TRACE))),
        "every span names the trace"
    );
    // Spans outside the task (the heartbeat) do not borrow its trace.
    let others: BTreeSet<&str> = spans
        .iter()
        .filter(|s| s.attributes.get("task").is_none_or(String::is_empty))
        .map(|s| s.trace_id.as_str())
        .collect();
    assert!(!others.contains(TRACE), "{others:?}");
}

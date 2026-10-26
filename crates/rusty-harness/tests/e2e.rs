//! Feature: The definition-of-done demo (T4)
//! Requirement: docs/specs/issue-3/requirements.md#R32
//!
//! One harness from `tests/fixtures/demo/config*.toml`, the real model (`gpt-6.1-sol`,
//! key by reference) and, for `remote`, Temporal Cloud; the TUI and a second A2A client
//! on it at once. Ignored by default: they need credentials and spend tokens.
//!
//! ```sh
//! cargo nextest run -p rusty_harness --features cli -E 'test(e2e_none_)' --run-ignored all
//! cargo nextest run -p rusty_harness --features cli -E 'test(e2e_remote_)' --run-ignored all
//! ```
//!
//! With `RH_E2E_EVIDENCE=<dir>` each run writes its TUI frames, the task and the exported
//! spans there (no credential is in any of them; the Temporal address is not either).
#![cfg(feature = "cli")]

use core::fmt::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt as _;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message as _;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rusty_harness::HarnessBuilder;
use rusty_harness::app_service::a2a_server::serve;
use rusty_harness::app_service::o11y::{self, Scope};
use rusty_harness::config::{Config, DurableMode};
use rusty_harness::harness::a2a_client::AgentTransport as _;
use rusty_harness::harness::a2a_client::official::{OfficialTransport, fetch_card};
use rusty_harness::harness::core_loop::task::TASK_EXT;
use rusty_harness::host::Host;
use rusty_harness::interaction::renderer::tui::{App, Footer, Outgoing, Theme, TuiOptions, view};
use serde_json::{Value, json};

/// A turn's budget: one or two model calls on a reasoning model.
const TURN: Duration = Duration::from_secs(240);

type Spans = Arc<Mutex<Vec<Value>>>;

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// The OTLP endpoint the harness exports to: each span flattened to JSON.
async fn collector() -> (SocketAddr, Spans) {
    async fn ingest(State(spans): State<Spans>, body: axum::body::Bytes) -> http::StatusCode {
        let Ok(request) = ExportTraceServiceRequest::decode(body) else {
            return http::StatusCode::BAD_REQUEST;
        };
        let mut out = spans.lock().unwrap();
        for resource in request.resource_spans {
            for scope in resource.scope_spans {
                for span in scope.spans {
                    let attributes: serde_json::Map<String, Value> = span
                        .attributes
                        .iter()
                        .map(|kv| {
                            let value = kv.value.as_ref().and_then(|v| v.value.as_ref()).map_or(
                                Value::Null,
                                |v| match v {
                                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s) => json!(s),
                                    opentelemetry_proto::tonic::common::v1::any_value::Value::IntValue(i) => json!(i),
                                    opentelemetry_proto::tonic::common::v1::any_value::Value::BoolValue(b) => json!(b),
                                    other => json!(format!("{other:?}")),
                                },
                            );
                            (kv.key.clone(), value)
                        })
                        .collect();
                    out.push(json!({
                        "name": span.name,
                        "trace_id": hex(&span.trace_id),
                        "span_id": hex(&span.span_id),
                        "parent_span_id": hex(&span.parent_span_id),
                        "start_unix_nano": span.start_time_unix_nano,
                        "end_unix_nano": span.end_time_unix_nano,
                        "attributes": attributes,
                    }));
                }
            }
        }
        http::StatusCode::OK
    }
    let spans: Spans = Arc::default();
    let app = axum::Router::new()
        .route("/v1/traces", axum::routing::post(ingest))
        .with_state(Arc::clone(&spans));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, spans)
}

/// The demo configuration as checked in, with what a test must own: a free port, a fresh
/// store, the plugin directory by absolute path and the collector.
fn demo_config(file: &str, collector: SocketAddr, data: &Path) -> Config {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(root.join("tests/fixtures/demo").join(file)).unwrap();
    let overrides: toml::Table = toml::from_str(&format!(
        r#"
        [server]
        bind = "127.0.0.1:0"
        [persistence]
        path = {data:?}
        [plugins]
        paths = [{plugins:?}]
        [o11y]
        otlp_endpoint = "http://{collector}/"
        "#,
        data = data.display().to_string(),
        plugins = root
            .join("tests/fixtures/demo/plugins")
            .display()
            .to_string(),
    ))
    .unwrap();
    Config::from_layers(Some(&text), Vec::new(), Some(overrides)).expect("the demo config loads")
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

fn frame(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(110, 40)).unwrap();
    terminal
        .draw(|f| view::draw_with(app, f, Theme::mono()))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..40u16 {
        let line: String = (0..110u16)
            .map(|x| buffer.cell((x, y)).unwrap().symbol().to_owned())
            .collect();
        text.push_str(line.trim_end());
        text.push('\n');
    }
    text
}

/// Sends what the TUI produced and feeds every event back to it.
async fn drive(app: &mut App, transport: &OfficialTransport, outgoing: Vec<Outgoing>) {
    for out in outgoing {
        let Outgoing::Send(message) = out else {
            continue;
        };
        let request = a2a::SendMessageRequest {
            message: *message,
            configuration: None,
            metadata: None,
            tenant: None,
        };
        let mut stream = transport
            .send_streaming_message(request, Vec::new())
            .await
            .expect("stream");
        tokio::time::timeout(TURN, async {
            while let Some(item) = stream.next().await {
                app.apply(item.expect("an event"), now());
            }
        })
        .await
        .expect("the turn ends in time");
    }
}

/// What a turn must leave behind: the task in `state`, waiting `why`.
async fn task(transport: &OfficialTransport, id: &str) -> a2a::Task {
    transport
        .get_task(a2a::GetTaskRequest {
            id: id.into(),
            history_length: None,
            tenant: None,
        })
        .await
        .expect("the task")
}

fn text_of(task: &a2a::Task) -> String {
    serde_json::to_string(task).unwrap_or_default()
}

struct Evidence {
    dir: Option<PathBuf>,
    variant: &'static str,
    console: String,
}

impl Evidence {
    fn new(variant: &'static str) -> Self {
        let dir = std::env::var_os("RH_E2E_EVIDENCE").map(PathBuf::from);
        if let Some(dir) = &dir {
            std::fs::create_dir_all(dir).unwrap();
        }
        Self {
            dir,
            variant,
            console: String::new(),
        }
    }

    fn step(&mut self, title: &str, app: &mut App) -> String {
        let shown = frame(app);
        let _ = write!(self.console, "## {title}\n\n```text\n{shown}```\n\n");
        shown
    }

    fn write(&self, name: &str, contents: &str) {
        if let Some(dir) = &self.dir {
            std::fs::write(dir.join(format!("{}-{name}", self.variant)), contents).unwrap();
        }
    }
}

/// The journey both variants run (R32.1–3): turn 1 from the TUI asks for the audience
/// through an A2UI form; the TUI answers it; turn 3 comes from a second client; the
/// task stays open, `input-required` awaiting the next turn, throughout.
#[allow(clippy::too_many_lines)] // one journey, read top to bottom
async fn the_demo(variant: &'static str, file: &str, mode: DurableMode) {
    let (collector_addr, spans) = collector().await;
    let data = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("demo-{variant}-{}", uuid::Uuid::new_v4()));
    let config = demo_config(file, collector_addr, &data);
    assert_eq!(config.durable.mode, mode);
    let host = Host::native();
    let _guard = o11y::init(&config.o11y, &host, Scope::Global).expect("the subscriber");
    let harness = HarnessBuilder::new(config)
        .host(host)
        .build()
        .await
        .expect("the demo builds");
    if mode == DurableMode::Remote {
        assert!(
            harness.temporal().is_some(),
            "remote runs on Temporal Cloud"
        );
    }
    let server = serve(&harness, None).await.expect("binds");
    let base: url::Url = format!("http://{}/", server.addr).parse().unwrap();
    tokio::spawn(server.run(std::future::pending()));

    let client_host = Host::native();
    let card = fetch_card(&client_host, &base).await.expect("the card");
    // The TUI and a second client (the web renderer's protocol leg) on the one harness.
    let tui_transport = OfficialTransport::from_card(&card, None, Vec::new()).expect("transport");
    let web_transport = OfficialTransport::from_card(&card, None, Vec::new()).expect("transport");
    let mut app = App::new(TuiOptions {
        agent: "demo-agent".into(),
        footer: Footer {
            model: "default".into(),
            effort: Some("medium".into()),
            durable: format!("{mode:?}").to_lowercase(),
        },
        ..TuiOptions::default()
    });
    app.adopt_card(&card);
    let mut evidence = Evidence::new(variant);

    // Turn 1 (TUI): the model asks for the audience through an A2UI form on a channel.
    for c in "Draft the release notes for rusty-harness 0.2: the npm and PyPI packages, the Responses API, voice. Keep the task open.".chars() {
        app.key(key(KeyCode::Char(c)), now());
    }
    let out = app.key(key(KeyCode::Enter), now());
    drive(&mut app, &tui_transport, out).await;
    let shown = evidence.step(
        "Turn 1: the TUI asks; the model asks back through A2UI",
        &mut app,
    );
    assert!(
        shown.contains("a2ui · basic catalog"),
        "the A2UI form renders in the TUI:\n{shown}"
    );
    assert!(
        shown.contains("Which audience are these notes for?"),
        "{shown}"
    );
    assert!(
        shown.contains("input-required awaiting participant"),
        "{shown}"
    );
    let task_id = app
        .task
        .as_ref()
        .expect("the TUI holds the task")
        .id
        .clone();
    let seen_by_web = task(&web_transport, &task_id).await;
    assert_eq!(
        seen_by_web.status.state,
        a2a::TaskState::InputRequired,
        "the other client sees it waiting"
    );

    // Turn 2 (TUI answers the form): the model drafts for the chosen audience.
    app.key(key(KeyCode::Tab), now());
    app.key(key(KeyCode::Tab), now());
    let out = app.key(key(KeyCode::Enter), now());
    assert_eq!(out.len(), 1, "one answer on the channel: {out:?}");
    drive(&mut app, &tui_transport, out).await;
    let shown = evidence.step("Turn 2: the form is answered; the model drafts", &mut app);
    assert!(
        shown.contains("input-required awaiting next turn"),
        "the task stays open:\n{shown}"
    );

    // Turn 3 (the second client, as the web renderer sends it): a revision.
    let open = task(&web_transport, &task_id).await;
    let mut message = a2a::Message::new(
        a2a::Role::User,
        vec![a2a::Part::text("Shorter, please: three bullets at most.")],
    );
    message.task_id = Some(task_id.clone());
    message.context_id = Some(open.context_id.clone());
    message.metadata = Some([(TASK_EXT.to_owned(), json!({ "mode": "long-running" }))].into());
    let request = a2a::SendMessageRequest {
        message,
        configuration: None,
        metadata: None,
        tenant: None,
    };
    let mut stream = web_transport
        .send_streaming_message(request, Vec::new())
        .await
        .expect("stream");
    tokio::time::timeout(TURN, async { while stream.next().await.is_some() {} })
        .await
        .expect("turn 3 ends in time");
    let after = task(&web_transport, &task_id).await;
    assert_eq!(
        after.status.state,
        a2a::TaskState::InputRequired,
        "still open after three turns"
    );
    // The TUI sees the other client's turn on the same task.
    app.apply(a2a::StreamResponse::Task(after.clone()), now());
    let shown = evidence.step(
        "Turn 3: the second client revises; the TUI shows it",
        &mut app,
    );
    assert!(
        shown.contains("input-required awaiting next turn"),
        "{shown}"
    );

    // R32.6: the task, its trace and (remote) the workflow id for its history.
    tokio::time::sleep(Duration::from_secs(6)).await; // the batch exporter's interval
    let spans = spans.lock().unwrap().clone();
    assert!(!spans.is_empty(), "the task's spans reached the collector");
    let text = text_of(&after);
    assert!(!text.contains("sk-"), "no credential in the task");
    evidence.write("console.md", &format!("{}\n", evidence.console.trim_end()));
    evidence.write("task.json", &serde_json::to_string_pretty(&after).unwrap());
    evidence.write("trace.json", &serde_json::to_string_pretty(&spans).unwrap());
    evidence.write("workflow-id.txt", &task_id);
    let _ = std::fs::remove_dir_all(&data);
}

/// Scenario: The demo runs end to end in none mode
///     Given the demo configuration, `OpenAI` GPT 6.1 by reference and `none` durability
///     When the TUI and a second client work one long-running task over three turns
///     Then the model asks through an A2UI form, the answer resumes it, and it stays open
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs OPENAI_API_KEY; spends tokens"]
async fn e2e_none_the_demo_runs_end_to_end() {
    Box::pin(the_demo("none", "config.toml", DurableMode::None)).await;
}

/// Scenario: The demo runs end to end in remote mode
///     Given the same, on Temporal Cloud (address, namespace and key by reference)
///     When the same three turns run
///     Then the same holds, and the task is a workflow on Temporal Cloud
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs OPENAI_API_KEY and Temporal Cloud; spends tokens"]
async fn e2e_remote_the_demo_runs_end_to_end() {
    Box::pin(the_demo(
        "remote",
        "config.remote.toml",
        DurableMode::Remote,
    ))
    .await;
}

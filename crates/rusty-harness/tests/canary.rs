//! Feature: Credential handling
//! Requirement: docs/specs/issue-3/requirements.md#security-considerations (abuse 7)
//!
//! The second leg of the canary: the demo path end to end (real host, real files, the
//! provider over HTTP, the A2A server over TCP) with a canary model key and a canary
//! bearer token, looking for either in every A2A response, the card, the store on
//! disk, the logs and spans, and the provider error path.
#![cfg(feature = "server")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::State;
use axum::http::HeaderMap;
use rusty_harness::HarnessBuilder;
use rusty_harness::app_service::a2a_server::serve;
use rusty_harness::app_service::o11y::RecordingLayer;
use rusty_harness::config::{Config, ConfigSources};
use rusty_harness::host::Host;
use tracing_subscriber::layer::SubscriberExt as _;

const KEY: &str = "sk-canary-5f0e1d9c7b";
const TOKEN: &str = "tok-canary-9d3a2b1c";

/// A provider that answers once, then refuses with a body echoing the credential it was
/// given, as a hostile or careless provider might.
async fn provider(
    State(calls): State<Arc<AtomicUsize>>,
    headers: HeaderMap,
) -> axum::response::Response {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let n = calls.fetch_add(1, Ordering::SeqCst);
    if n == 0 {
        axum::response::IntoResponse::into_response((
            http::StatusCode::OK,
            [("content-type", "text/event-stream")],
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"the answer\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\n",
                "data: [DONE]\n\n"
            ),
        ))
    } else {
        axum::response::IntoResponse::into_response((
            http::StatusCode::UNAUTHORIZED,
            [("content-type", "application/json")],
            format!("{{\"error\":{{\"message\":\"bad credential: {auth}\"}}}}"),
        ))
    }
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

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files_under(&path, out);
            } else {
                out.push(path);
            }
        }
    }
}

async fn rpc(client: &reqwest::Client, base: &str, body: serde_json::Value) -> String {
    client
        .post(base)
        .bearer_auth(TOKEN)
        .json(&body)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

fn message(text: &str, task: Option<&str>) -> serde_json::Value {
    let mut m = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text(text)]);
    m.task_id = task.map(str::to_owned);
    serde_json::to_value(a2a::SendMessageRequest {
        message: m,
        configuration: None,
        metadata: None,
        tenant: None,
    })
    .unwrap()
}

/// Scenario: The canary credential appears nowhere observable
///     Given a model key and a bearer token that are canary values, referenced by file
///     When the agent card, every A2A operation, a streaming turn, an unauthenticated
///     request and a provider refusal that echoes the key are exercised
///     Then neither value appears in any response, the card, a store file, a log or a span
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn abuse_canary_credential_appears_nowhere_observable_end_to_end() {
    let recording = RecordingLayer::default();
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(recording.clone()))
        .expect("first subscriber in this process");

    let dir =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("canary-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let key_file = dir.join("openai.key");
    let token_file = dir.join("bearer.token");
    std::fs::write(&key_file, KEY).unwrap();
    std::fs::write(&token_file, TOKEN).unwrap();
    let store_dir = dir.join("data");

    let calls = Arc::new(AtomicUsize::new(0));
    let app = axum::Router::new()
        .route("/v1/chat/completions", axum::routing::post(provider))
        .with_state(Arc::clone(&calls));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let overrides: toml::Table = toml::from_str(&format!(
        r#"
[models.default]
provider = "openai"
model = "mock"
api_key = "file:{key}"
base_url = "http://{provider_addr}/v1"
[models.default.retry]
max_attempts = 1
[server]
bind = "127.0.0.1:0"
[server.security]
scheme = "bearer"
token = "file:{token}"
[persistence]
store = "file"
path = "{store}"
"#,
        key = key_file.display(),
        token = token_file.display(),
        store = store_dir.display(),
    ))
    .unwrap();
    let config = Config::load(
        &ConfigSources {
            file: None,
            env: false,
            overrides: Some(overrides),
        },
        &rusty_harness::host::TokioFs,
    )
    .await
    .expect("config");
    assert!(!format!("{config:?}").contains(KEY), "config debug");

    let h = HarnessBuilder::new(config)
        .host(Host::native())
        .build()
        .await
        .expect("builds");
    let server = serve(&h, None).await.expect("binds");
    let base = format!("http://{}/", server.addr);
    tokio::spawn(server.run(std::future::pending()));
    let client = a2a_client();
    let mut observed: Vec<(String, String)> = Vec::new();
    let card = client
        .get(format!("{base}.well-known/agent-card.json"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        card.contains("securitySchemes") || card.contains("security"),
        "{card}"
    );
    observed.push(("card".into(), card));
    let unauthenticated = client
        .post(&base)
        .json(&serde_json::json!({"jsonrpc":"2.0","id":0,"method":"ListTasks","params":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), http::StatusCode::UNAUTHORIZED);
    observed.push((
        "unauthenticated".into(),
        unauthenticated.text().await.unwrap(),
    ));
    let wrong = client
        .post(&base)
        .bearer_auth("tok-wrong")
        .json(&serde_json::json!({"jsonrpc":"2.0","id":0,"method":"ListTasks","params":{}}))
        .send()
        .await
        .unwrap();
    observed.push(("wrong token".into(), wrong.text().await.unwrap()));
    let sent = rpc(&client, &base, serde_json::json!({"jsonrpc":"2.0","id":1,"method":"SendMessage","params": message("hello", None)})).await;
    let sent_json: serde_json::Value = serde_json::from_str(&sent).unwrap();
    let task_id = sent_json["result"]["task"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{sent}"))
        .to_owned();
    assert!(sent.contains("the answer"), "{sent}");
    observed.push(("SendMessage".into(), sent));
    observed.push((
        "GetTask".into(),
        rpc(
            &client,
            &base,
            serde_json::json!({"jsonrpc":"2.0","id":2,"method":"GetTask","params":{"id": task_id}}),
        )
        .await,
    ));
    observed.push((
        "ListTasks".into(),
        rpc(
            &client,
            &base,
            serde_json::json!({"jsonrpc":"2.0","id":3,"method":"ListTasks","params":{}}),
        )
        .await,
    ));

    // The provider now refuses and echoes the key; the failure reaches the task.
    let failed = rpc(&client, &base, serde_json::json!({"jsonrpc":"2.0","id":4,"method":"SendMessage","params": message("again", None)})).await;
    assert!(
        failed.contains("TASK_STATE_FAILED")
            || failed.contains("failed")
            || failed.contains("error"),
        "{failed}"
    );
    observed.push(("SendMessage (provider refused)".into(), failed));
    let streamed = client
        .post(&base)
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"jsonrpc":"2.0","id":5,"method":"SendStreamingMessage","params": message("once more", None)}))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    observed.push(("SendStreamingMessage".into(), streamed));
    assert!(calls.load(Ordering::SeqCst) >= 3, "the provider was called");

    for (what, body) in &observed {
        assert!(!body.contains(KEY), "{what} leaks the model key: {body}");
        assert!(
            !body.contains(TOKEN),
            "{what} leaks the bearer token: {body}"
        );
    }
    let mut files = Vec::new();
    files_under(&store_dir, &mut files);
    assert!(
        !files.is_empty(),
        "the file store wrote under {}",
        store_dir.display()
    );
    for file in &files {
        let bytes = std::fs::read(file).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains(KEY) && !text.contains(TOKEN),
            "{} leaks a credential",
            file.display()
        );
    }
    let logs = format!("{:?} {:?}", recording.events(), recording.spans());
    assert!(
        !logs.contains(KEY) && !logs.contains(TOKEN),
        "logs or spans leak a credential: {logs}"
    );
    assert!(
        recording.spans().iter().any(|s| s.point == "model.call"),
        "the model calls were spanned"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

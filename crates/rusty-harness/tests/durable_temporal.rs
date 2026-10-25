//! Feature: Durable execution on Temporal
//! Requirement: docs/specs/issue-3/requirements.md#R22
//!
//! The `embedded` mode: the SDK's ephemeral dev server (the `temporal` CLI, downloaded
//! once into `~/.cache/rusty-harness/temporal/`), a worker in this process, and the
//! data converter that keeps secrets out of payloads (boundary 6).
#![cfg(feature = "durable-embedded")]

use std::fmt::Write as _;
use std::sync::Arc;

use futures::StreamExt as _;
use rusty_harness::app_service::durable::temporal::{
    PingActivities, PingWorkflow, TemporalConnection, TemporalWorker, ping_worker_options,
};
use rusty_harness::config::{DurableConfig, DurableMode};
use rusty_harness::host::testing::MapSecrets;
use rusty_harness::host::{Secret, SecretRef};
use temporalio_client::{
    WorkflowFetchHistoryOptions, WorkflowGetResultOptions, WorkflowStartOptions,
};

const CANARY: &str = "tmprl-canary-0c1d2e";

/// The payload bytes an event carries (inputs and results), decoded from the proto.
fn payload_bytes(
    event: &temporalio_common::protos::temporal::api::history::v1::HistoryEvent,
) -> Vec<u8> {
    use temporalio_common::protos::temporal::api::history::v1::history_event::Attributes as A;
    let payloads = match event.attributes.as_ref() {
        Some(A::WorkflowExecutionStartedEventAttributes(a)) => a.input.clone(),
        Some(A::ActivityTaskScheduledEventAttributes(a)) => a.input.clone(),
        Some(A::ActivityTaskCompletedEventAttributes(a)) => a.result.clone(),
        Some(A::WorkflowExecutionCompletedEventAttributes(a)) => a.result.clone(),
        _ => None,
    };
    payloads
        .map(|p| p.payloads.into_iter().flat_map(|p| p.data).collect())
        .unwrap_or_default()
}

fn cache_dir() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join(".cache/rusty-harness/temporal")
}

async fn embedded() -> TemporalConnection {
    let mut config = DurableConfig {
        mode: DurableMode::Embedded,
        ..Default::default()
    };
    config.temporal.task_queue = format!("rh-test-{}", uuid::Uuid::new_v4());
    TemporalConnection::connect(&config, &MapSecrets::default(), Some(cache_dir()))
        .await
        .expect("the embedded dev server starts")
}

/// Scenario: The embedded dev server starts and a trivial workflow completes
///     Given mode embedded and no Temporal installed
///     When the harness starts the dev server, a worker and the ping workflow
///     Then the workflow completes with the activity's answer and the server shuts down
/// Scenario: The canary credential appears nowhere observable (Temporal leg)
///     Given an activity instance holding a resolved secret and a guard protecting it
///     When a workflow is started with a reference as input, and another with the value
///     Then the history carries the reference and never the value, and the second start
///     is refused before anything reaches the service
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_embedded_dev_server_starts_and_a_trivial_workflow_completes() {
    let connection = embedded().await;
    assert_eq!(connection.mode(), DurableMode::Embedded);
    let secret = Arc::new(Secret::new(CANARY.to_owned()));
    connection.guard().protect(&secret);
    let worker = TemporalWorker::start(
        &connection,
        ping_worker_options(
            &connection,
            PingActivities {
                held_secret: Some(secret),
            },
        )
        .unwrap(),
    )
    .expect("worker");

    let handle = connection
        .client()
        .start_workflow(
            PingWorkflow::run,
            SecretRef::Env("OPENAI_API_KEY".into()).to_string(),
            WorkflowStartOptions::new(connection.task_queue().to_owned(), "ping-1".to_owned())
                .build(),
        )
        .await
        .expect("starts");
    let result = handle
        .get_result(WorkflowGetResultOptions::default())
        .await
        .expect("completes");
    assert_eq!(result, "pong: env:OPENAI_API_KEY (holding a credential)");

    // abuse_temporal_payloads_carry_no_secret: the history holds the reference, never
    // the value; a payload that would carry the value is refused client-side.
    let mut history = handle.fetch_history(WorkflowFetchHistoryOptions::default());
    let mut dump = String::new();
    let mut payloads: Vec<u8> = Vec::new();
    while let Some(event) = history.next().await {
        let event = event.expect("event");
        payloads.extend(payload_bytes(&event));
        let _ = writeln!(dump, "{event:?}");
    }
    let payloads = String::from_utf8_lossy(&payloads);
    assert!(
        payloads.contains("env:OPENAI_API_KEY"),
        "the reference travels: {payloads}"
    );
    assert!(
        !payloads.contains(CANARY),
        "a payload carries the credential: {payloads}"
    );
    assert!(!dump.contains(CANARY), "the history carries the credential");
    let refused = connection
        .client()
        .start_workflow(
            PingWorkflow::run,
            format!("key {CANARY}"),
            WorkflowStartOptions::new(connection.task_queue().to_owned(), "ping-2".to_owned())
                .build(),
        )
        .await;
    let Err(err) = refused else {
        panic!("a payload with the credential must be refused");
    };
    assert!(err.to_string().contains("credential"), "{err}");
    assert!(!err.to_string().contains(CANARY));

    worker.stop().await.expect("worker stops");
    connection.shutdown().await;
}

#[allow(dead_code)]
fn abuse_temporal_payloads_carry_no_secret() {
    // The scenario above carries this name's assertions; it keeps one dev server per test
    // binary.
}

mod task_workflow {
    //! `TaskWorkflow` under the embedded mode (task 29): the three-turn scenario and the
    //! attempt protocol, driven over the A2A server like any client would.
    use std::sync::Arc;

    use rusty_harness::HarnessBuilder;
    use rusty_harness::app_service::a2a_server::serve;
    use rusty_harness::app_service::durable::temporal::workflow::TaskWorkflowRun;
    use rusty_harness::app_service::durable::temporal::workflow::WorkflowOutcome;
    use rusty_harness::config::{Config, ConfigSources};
    use rusty_harness::harness::core_loop::task::{Caller, Role, TASK_EXT};
    use rusty_harness::harness::models::llm::testing::RecordedLlm;
    use rusty_harness::harness::models::llm::{ModelEvent, StopReason};
    use rusty_harness::host::Host;
    use temporalio_client::{WorkflowFetchHistoryOptions, WorkflowGetResultOptions};

    fn text(t: &str) -> Vec<ModelEvent> {
        vec![
            ModelEvent::TextDelta { text: t.into() },
            ModelEvent::Stop {
                reason: StopReason::EndTurn,
            },
        ]
    }

    async fn embedded_config() -> Config {
        let overrides: toml::Table = toml::from_str(&format!(
            r#"
[server]
bind = "127.0.0.1:0"
[durable]
mode = "embedded"
[durable.temporal]
task_queue = "rh-task-{}"
[durable.retry]
initial = "10ms"
coefficient = 2
max_interval = "100ms"
max_attempts = 3
"#,
            uuid::Uuid::new_v4()
        ))
        .unwrap();
        Config::load(
            &ConfigSources {
                file: None,
                env: false,
                overrides: Some(overrides),
            },
            &rusty_harness::host::TokioFs,
        )
        .await
        .expect("config")
    }

    fn client() -> reqwest::Client {
        let mut headers = http::HeaderMap::new();
        headers.insert("A2A-Version", http::HeaderValue::from_static("1.0"));
        reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap()
    }

    fn message(text: &str, task: Option<&str>, long_running: bool) -> serde_json::Value {
        let mut m = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text(text)]);
        m.task_id = task.map(str::to_owned);
        if long_running {
            m.metadata = Some(std::collections::HashMap::from([(
                TASK_EXT.to_owned(),
                serde_json::json!({ "mode": "long-running" }),
            )]));
        }
        serde_json::json!({ "message": serde_json::to_value(&m).unwrap() })
    }

    async fn rpc(
        client: &reqwest::Client,
        base: &str,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        client
            .post(base)
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    /// Scenario: A long-running task stays in input-required across three turns (embedded)
    ///     Given the harness in durable mode embedded, serving A2A
    ///     When a client sends three turns to one long-running task and then cancels it
    ///     Then each turn's answer comes back through the workflow's activities, the task
    ///     waits between turns, and the cancel ends it canceled
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_long_running_task_runs_three_turns_as_a_temporal_workflow() {
        let h = HarnessBuilder::new(embedded_config().await)
            .host(Host::testing())
            .llm(Arc::new(RecordedLlm::new(
                "default",
                vec![text("first"), text("second"), text("third")],
            )))
            .build()
            .await
            .expect("builds against the embedded dev server");
        assert!(h.temporal().is_some(), "the Temporal mode is on");
        let server = serve(&h, None).await.expect("binds");
        let base = format!("http://{}/", server.addr);
        tokio::spawn(server.run(std::future::pending()));
        let client = client();

        let first = rpc(&client, &base, "SendMessage", message("hello", None, true)).await;
        let task = first["result"]["task"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{first}"))
            .to_owned();
        assert_eq!(
            first["result"]["task"]["status"]["state"], "TASK_STATE_INPUT_REQUIRED",
            "{first}"
        );
        assert_eq!(
            first["result"]["task"]["status"]["message"]["parts"][0]["text"],
            "first"
        );
        for (i, expected) in ["second", "third"].iter().enumerate() {
            let turn = rpc(
                &client,
                &base,
                "SendMessage",
                message("again", Some(&task), false),
            )
            .await;
            assert_eq!(
                turn["result"]["task"]["status"]["message"]["parts"][0]["text"],
                *expected,
                "turn {}: {turn}",
                i + 2
            );
            assert_eq!(
                turn["result"]["task"]["status"]["state"],
                "TASK_STATE_INPUT_REQUIRED"
            );
        }
        let canceled = rpc(
            &client,
            &base,
            "CancelTask",
            serde_json::json!({"id": task}),
        )
        .await;
        assert_eq!(
            canceled["result"]["status"]["state"], "TASK_STATE_CANCELED",
            "{canceled}"
        );

        // The workflow ends with the task (the cancel signal wakes it); its history is
        // the replayer fixture (T1).
        let handle = h
            .temporal()
            .unwrap()
            .connection()
            .client()
            .get_workflow_handle::<TaskWorkflowRun>(task.clone());
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            handle.get_result(WorkflowGetResultOptions::default()),
        )
        .await
        .expect("the workflow ends within 30 s")
        .expect("the workflow completed");
        assert_eq!(outcome, WorkflowOutcome::Canceled);
        let events = handle
            .fetch_history(WorkflowFetchHistoryOptions::default())
            .into_events()
            .await
            .expect("history");
        assert!(events.len() > 20, "{} events", events.len());
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            let json = handle
                .fetch_history(WorkflowFetchHistoryOptions::default())
                .to_json()
                .await
                .expect("history json");
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/temporal/task_workflow_history.json");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, json).unwrap();
        }
    }

    /// Scenario: A retried model call retracts the partial output of the failed attempt
    ///     Given a model whose first call fails with a retryable error
    ///     When the workflow retries the `call_model` activity
    ///     Then a status update retracting attempt 1 is committed before the answer of
    ///     attempt 2, and the task completes with that answer
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_retried_model_call_retracts_the_failed_attempt() {
        let h = HarnessBuilder::new(embedded_config().await)
            .host(Host::testing())
            .llm(Arc::new(RecordedLlm::failing_then(
                "default",
                1,
                true,
                text("recovered"),
            )))
            .build()
            .await
            .expect("builds");
        let server = serve(&h, None).await.expect("binds");
        let base = format!("http://{}/", server.addr);
        tokio::spawn(server.run(std::future::pending()));
        let client = client();
        let sent = rpc(&client, &base, "SendMessage", message("go", None, false)).await;
        assert_eq!(
            sent["result"]["task"]["status"]["state"], "TASK_STATE_COMPLETED",
            "{sent}"
        );
        assert_eq!(
            sent["result"]["task"]["status"]["message"]["parts"][0]["text"],
            "recovered"
        );
        let task = sent["result"]["task"]["id"].as_str().unwrap().to_owned();

        let viewer = Caller::human("operator", [Role::Admin]);
        let events = h
            .store()
            .events(&task, 1, usize::MAX, &viewer)
            .await
            .unwrap();
        let retracted = events.iter().position(|e| match &e.response {
            a2a::StreamResponse::StatusUpdate(u) => u
                .metadata
                .as_ref()
                .and_then(|m| m.get(TASK_EXT))
                .and_then(|t| t.get("retracted"))
                .is_some_and(|r| r == &serde_json::json!([{ "iteration": 1, "n": 1 }])),
            _ => false,
        });
        let answer = events.iter().position(|e| match &e.response {
            a2a::StreamResponse::Message(m) => m.text() == Some("recovered"),
            _ => false,
        });
        assert!(
            retracted.is_some(),
            "a retraction of attempt 1: {events:#?}"
        );
        assert!(answer.is_some(), "the answer of attempt 2");
        assert!(
            retracted < answer,
            "the retraction precedes the retried answer"
        );
    }
}

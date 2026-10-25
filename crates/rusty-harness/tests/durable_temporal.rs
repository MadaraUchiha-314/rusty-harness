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

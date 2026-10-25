//! Feature: Durable execution on Temporal, crash recovery
//! Requirement: docs/specs/issue-3/requirements.md#R22 (R22.5), R23.4
//!
//! The T13 chaos scenarios: two worker processes against one ephemeral dev server and one
//! file store; the test kills one with SIGKILL. The child is this test binary itself,
//! running an `#[ignore]`d test as the victim.
#![cfg(feature = "durable-embedded")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusty_harness::app_service::durable::temporal::{
    TemporalConnection, TemporalExecution, WorkerRoles,
};
use rusty_harness::app_service::durable::{
    ActivityFailed, ActivityRetry, ActivityTimeouts, DurableExecution, RetryDecision, RetryPolicy,
};
use rusty_harness::app_service::inbox::{EventRef, EventTarget};
use rusty_harness::config::{Config, ConfigSources, DurableConfig, DurableMode};
use rusty_harness::harness::core_loop::agent::{AgentRequest, CallContext};
use rusty_harness::harness::core_loop::task::{Caller, Role, TASK_EXT};
use rusty_harness::harness::hooks::{Hook, HookCx, HookOrder, InHook, Next};
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{
    EventStream, Llm, ModelCapabilities, ModelError, ModelEvent, ModelRequest, StopReason,
};
use rusty_harness::harness::tools::{
    Idempotency, Tool, ToolCx, ToolDescriptor, ToolError, ToolOutput,
};
use rusty_harness::host::{Host, SecretRef};
use rusty_harness::registry::{Entity, EntityId, EntityKind};
use rusty_harness::{Harness, HarnessBuilder};
use serde_json::json;

const DIR: &str = "RH_CHAOS_DIR";
const ADDRESS: &str = "RH_CHAOS_TEMPORAL";
const QUEUE: &str = "RH_CHAOS_QUEUE";
const EFFECTS: &str = "RH_CHAOS_EFFECTS";
const TASK: &str = "chaos-task";
const PEER_CARD: &str = "RH_CHAOS_PEER_CARD";

/// A model that takes its time: the window in which the worker is killed.
struct SlowLlm {
    id: EntityId,
    inner: RecordedLlm,
    delay: Duration,
}

impl Entity for SlowLlm {
    fn kind(&self) -> EntityKind {
        EntityKind::Llm
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

#[async_trait::async_trait]
impl Llm for SlowLlm {
    fn capabilities(&self) -> &ModelCapabilities {
        self.inner.capabilities()
    }
    async fn stream(&self, request: ModelRequest) -> Result<EventStream<'static>, ModelError> {
        tokio::time::sleep(self.delay).await;
        self.inner.stream(request).await
    }
}

/// A side-effecting tool: appends one line to a file (the duplicate-effect probe).
struct Append;
impl Entity for Append {
    fn kind(&self) -> EntityKind {
        EntityKind::Tool
    }
    fn entity_id(&self) -> &EntityId {
        static ID: std::sync::OnceLock<EntityId> = std::sync::OnceLock::new();
        ID.get_or_init(|| EntityId::new("append"))
    }
}
#[async_trait::async_trait]
impl Tool for Append {
    fn descriptor(&self) -> &ToolDescriptor {
        static D: std::sync::OnceLock<ToolDescriptor> = std::sync::OnceLock::new();
        D.get_or_init(|| ToolDescriptor {
            name: "append".into(),
            description: "appends a line".into(),
            parameters: json!({ "type": "object" }),
            idempotency: Idempotency::SideEffecting { accepts_key: true },
        })
    }
    async fn call(&self, _cx: &ToolCx, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        use std::io::Write as _;
        let path = std::env::var(EFFECTS).expect(EFFECTS);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .unwrap();
        writeln!(file, "appended").unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(ToolOutput::Value {
            value: json!("appended"),
        })
    }
}

fn text(t: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta { text: t.into() },
        ModelEvent::Stop {
            reason: StopReason::EndTurn,
        },
    ]
}

fn tool_turn() -> Vec<ModelEvent> {
    vec![
        ModelEvent::ToolCall {
            call: rusty_harness::harness::tools::ToolCall {
                id: "call-1".into(),
                name: "append".into(),
                arguments: json!({}),
            },
        },
        ModelEvent::Stop {
            reason: StopReason::ToolUse,
        },
    ]
}

/// The native host: real HTTP (the peer is reached over A2A), the environment for the
/// dev server's address (`RH_CHAOS_TEMPORAL`, set before any harness is built).
fn agent_call_turn() -> Vec<ModelEvent> {
    vec![
        ModelEvent::ToolCall {
            call: rusty_harness::harness::tools::ToolCall {
                id: "call-peer".into(),
                name: "agent.call".into(),
                arguments: json!({ "agent": "peer", "message": "do the thing", "wait": false }),
            },
        },
        ModelEvent::Stop {
            reason: StopReason::ToolUse,
        },
    ]
}

fn host(address: &str) -> Host {
    // SAFETY: set before any thread of this process reads it; the child inherits it.
    unsafe { std::env::set_var("RH_CHAOS_TEMPORAL", address) };
    Host::native()
}

/// The harness over the shared file store in `none` mode; the Temporal side is attached
/// by hand so each process chooses its worker roles.
async fn harness(dir: &Path, llm: Arc<dyn Llm>, address: &str) -> Harness {
    let peer = std::env::var(PEER_CARD)
        .ok()
        .filter(|card| !card.is_empty())
        .map(|card| format!("[agents.peer]\ncard_url = \"{card}\"\n"))
        .unwrap_or_default();
    let overrides: toml::Table = toml::from_str(&format!(
        "[persistence]\nstore = \"file\"\npath = \"{}\"\n[loop]\nheartbeat_interval = \"300ms\"\n{peer}",
        dir.display()
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
    .unwrap();
    HarnessBuilder::new(config)
        .host(host(address))
        .llm(llm)
        .tool(Arc::new(Append))
        .build()
        .await
        .expect("builds")
}

fn durable_config(queue: &str) -> DurableConfig {
    let mut config = DurableConfig {
        mode: DurableMode::Remote,
        ..Default::default()
    };
    config.temporal.address = Some(SecretRef::Env("RH_CHAOS_TEMPORAL".into()));
    queue.clone_into(&mut config.temporal.task_queue);
    config.retry.initial = "50ms".parse().unwrap();
    config.retry.max_attempts = 3;
    config
}

fn fast_timeouts() -> ActivityTimeouts {
    ActivityTimeouts {
        short_ms: 20_000,
        model_ms: 60_000,
        model_heartbeat_ms: 1_500,
        tool_ms: 60_000,
        tool_heartbeat_ms: 1_500,
    }
}

async fn execution(h: &Harness, queue: &str) -> TemporalExecution {
    let secrets = h.host().secrets().unwrap();
    let connection = TemporalConnection::connect(&durable_config(queue), secrets.as_ref(), None)
        .await
        .expect("connects to the dev server");
    TemporalExecution::new(
        Arc::new(connection),
        Arc::clone(h.agent()),
        &RetryPolicy::from(&durable_config(queue).retry),
    )
    .with_timeouts(fast_timeouts())
    .with_heartbeat_every(Duration::from_millis(300))
}

/// The victim process: an activity-only or workflow-only worker, until killed.
#[test]
#[ignore = "spawned by the chaos scenarios"]
fn chaos_worker_child() {
    let dir = PathBuf::from(std::env::var(DIR).expect(DIR));
    let address = std::env::var(ADDRESS).expect(ADDRESS);
    let queue = std::env::var(QUEUE).expect(QUEUE);
    let role = std::env::var("RH_CHAOS_ROLE").expect("RH_CHAOS_ROLE");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let turns = if std::env::var(PEER_CARD).is_ok_and(|c| !c.is_empty()) {
            vec![agent_call_turn(), text("from child")]
        } else {
            vec![text("from child"), text("from child")]
        };
        let delay = if std::env::var(PEER_CARD).is_ok_and(|c| !c.is_empty()) {
            Duration::ZERO
        } else {
            Duration::from_secs(8)
        };
        let llm: Arc<dyn Llm> = Arc::new(SlowLlm {
            id: EntityId::new("default"),
            inner: RecordedLlm::new("default", turns),
            delay,
        });
        let h = harness(&dir, llm, &address).await;
        let execution = execution(&h, &queue).await;
        let roles = if role == "activities" {
            WorkerRoles::ActivitiesOnly
        } else {
            WorkerRoles::WorkflowsOnly
        };
        execution.start_worker_with(roles).expect("worker");
        std::future::pending::<()>().await;
    });
}

/// The victim, killed when the scenario ends however it ends (no orphan worker).
struct ChildGuard(std::process::Child);

impl ChildGuard {
    fn kill(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

fn spawn_child(dir: &Path, address: &str, queue: &str, role: &str, effects: &Path) -> ChildGuard {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "chaos_worker_child", "--ignored", "--nocapture"])
        .env(DIR, dir)
        .env(ADDRESS, address)
        .env(QUEUE, queue)
        .env("RH_CHAOS_ROLE", role)
        .env(EFFECTS, effects)
        .env(PEER_CARD, std::env::var(PEER_CARD).unwrap_or_default())
        .stdout(Stdio::null())
        // The child's stderr joins this test's: nextest keeps it when the test fails.
        .stderr(Stdio::inherit())
        .spawn()
        .map(ChildGuard)
        .unwrap()
}

/// The ephemeral dev server on a known port, so a second process can join it.
async fn dev_server() -> (
    temporalio_sdk::testing::WorkflowEnvironment<temporalio_sdk::testing::LocalServer>,
    String,
) {
    use temporalio_sdk::testing::{
        DevServerLogLevel, EphemeralExe, EphemeralExeVersion, LocalWorkflowEnvironmentOptions,
        WorkflowEnvironment,
    };
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let cache = std::env::var_os("HOME")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join(".cache/rusty-harness/temporal");
    std::fs::create_dir_all(&cache).unwrap();
    let env = WorkflowEnvironment::start_local(
        LocalWorkflowEnvironmentOptions::builder()
            .server_executable(EphemeralExe::CachedDownload {
                version: EphemeralExeVersion::Default,
                dest_dir: Some(cache.display().to_string()),
                ttl: None,
            })
            .port(port)
            .log_level(DevServerLogLevel::Warn)
            .build(),
    )
    .await
    .expect("dev server");
    (env, format!("http://127.0.0.1:{port}"))
}

async fn start_task(h: &Harness, execution: &TemporalExecution) {
    let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("go")]);
    message.task_id = Some(TASK.into());
    message.metadata = Some(HashMap::from([(
        TASK_EXT.to_owned(),
        json!({ "mode": "one-shot" }),
    )]));
    let request = AgentRequest {
        message,
        call: CallContext {
            task_id: TASK.into(),
            context_id: "chaos-ctx".into(),
            stored_task: None,
            caller: Caller::human("operator", [Role::Admin]),
            service_params: std::collections::BTreeMap::default(),
        },
        metadata: None,
        extensions: vec![],
        inbox_event_id: None,
        traceparent: None,
    };
    h.agent().open_task(&request).await.expect("task opened");
    execution
        .start_or_signal(
            &TASK.into(),
            EventRef {
                id: "chaos-event".into(),
                target: EventTarget::Task(TASK.into()),
            },
        )
        .await
        .expect("workflow started");
}

async fn wait_until(what: &str, limit: Duration, mut done: impl AsyncFnMut() -> bool) {
    let started = Instant::now();
    while !done().await {
        assert!(started.elapsed() < limit, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// What the workflow and the store say, for a failing scenario's output.
async fn diagnose(h: &Harness, execution: &TemporalExecution) -> String {
    use futures::StreamExt as _;
    use rusty_harness::app_service::durable::temporal::workflow::TaskWorkflowRun;
    use std::fmt::Write as _;
    let mut out = String::new();
    let record = h.store().get_task(&TASK.into()).await.unwrap();
    let _ = writeln!(out, "record: {record:?}");
    let handle = execution
        .connection()
        .client()
        .get_workflow_handle::<TaskWorkflowRun>(TASK.to_owned());
    let mut history =
        handle.fetch_history(temporalio_client::WorkflowFetchHistoryOptions::default());
    while let Some(event) = history.next().await {
        match event {
            Ok(e) => {
                let summary = format!("{:?}", e.attributes);
                let cut = summary
                    .char_indices()
                    .nth(500)
                    .map_or(summary.len(), |(i, _)| i);
                let _ = writeln!(out, "EVENT {:?} {}", e.event_type(), &summary[..cut]);
            }
            Err(e) => {
                let _ = writeln!(out, "history error: {e}");
            }
        }
    }
    out
}

/// Like [`wait_until`], printing the workflow's history when it gives up.
async fn wait_or_diagnose(
    what: &str,
    limit: Duration,
    h: &Harness,
    execution: &TemporalExecution,
    mut done: impl AsyncFnMut() -> bool,
) {
    let started = Instant::now();
    while !done().await {
        if started.elapsed() >= limit {
            let report = diagnose(h, execution).await;
            panic!("timed out waiting for {what}\n{report}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Records what the durable hook points saw.
#[derive(Default)]
struct Recorder {
    failures: Mutex<Vec<(String, bool)>>,
    decisions: Mutex<Vec<RetryDecision>>,
}

#[async_trait::async_trait]
impl InHook<ActivityFailed> for Recorder {
    async fn run<'a>(
        &self,
        _cx: &'a HookCx,
        input: rusty_harness::app_service::durable::ActivityFailure,
        next: Next<'a, ActivityFailed>,
    ) -> Result<(), rusty_harness::HarnessError> {
        self.failures
            .lock()
            .unwrap()
            .push((input.activity.activity.clone(), input.timeout));
        next(input).await
    }
}

#[async_trait::async_trait]
impl InHook<ActivityRetry> for Recorder {
    async fn run<'a>(
        &self,
        _cx: &'a HookCx,
        input: rusty_harness::app_service::durable::RetryProposal,
        next: Next<'a, ActivityRetry>,
    ) -> Result<RetryDecision, rusty_harness::HarnessError> {
        let decision = next(input).await?;
        self.decisions.lock().unwrap().push(decision.clone());
        Ok(decision)
    }
}

/// Scenario: Killing the activity worker re-dispatches the model call once
///     Given a workflow-only worker here and an activity-only worker in a child process
///     When the child is killed while its model call is in flight
///     Then the heartbeat timeout surfaces the crash, `retry_decision` runs on this
///     worker (its failure and retry hooks see a timeout), attempt 1 is retracted and the
///     model call runs once more, here, to completion
/// Scenario: A crashed activity's failure and retry hooks run on the surviving worker
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn chaos_killing_the_activity_worker_redispatches_the_model_call_once() {
    let (env, address) = dev_server().await;
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("chaos-activity-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let effects = dir.join("effects.log");
    let queue = format!("chaos-{}", uuid::Uuid::new_v4());
    let llm: Arc<dyn Llm> = Arc::new(RecordedLlm::new("default", vec![text("from survivor")]));
    let h = harness(&dir, llm, &address).await;
    let recorder = Arc::new(Recorder::default());
    h.hooks().register::<ActivityFailed>(
        EntityId::new("chaos.recorder.failed"),
        Hook::In(Arc::clone(&recorder) as Arc<dyn InHook<ActivityFailed>>),
        HookOrder::default(),
    );
    h.hooks().register::<ActivityRetry>(
        EntityId::new("chaos.recorder.retry"),
        Hook::In(Arc::clone(&recorder) as Arc<dyn InHook<ActivityRetry>>),
        HookOrder::default(),
    );
    let execution = execution(&h, &queue).await;
    execution
        .start_worker_with(WorkerRoles::WorkflowsOnly)
        .unwrap();
    let mut child = spawn_child(&dir, &address, &queue, "activities", &effects);

    start_task(&h, &execution).await;
    // The child built the context and is inside its slow model call.
    let store = Arc::clone(h.store());
    wait_until(
        "the child's build_context",
        Duration::from_secs(60),
        async || {
            store
                .get_blob(&format!("ctx/{TASK}/1"))
                .await
                .unwrap()
                .is_some()
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    child.kill();

    // The survivor takes the activities; the lost attempt times out on its heartbeat.
    execution
        .start_worker_with(WorkerRoles::ActivitiesOnly)
        .unwrap();
    wait_or_diagnose(
        "the task to complete",
        Duration::from_secs(45),
        &h,
        &execution,
        async || {
            store
                .get_task(&TASK.into())
                .await
                .unwrap()
                .is_some_and(|t| t.is_terminal())
        },
    )
    .await;
    let record = store.get_task(&TASK.into()).await.unwrap().unwrap();
    assert_eq!(record.state, a2a::TaskState::Completed, "{record:?}");
    assert_eq!(
        record.status_message.as_ref().and_then(|m| m.text()),
        Some("from survivor")
    );

    let viewer = Caller::human("operator", [Role::Admin]);
    let events = store
        .events(&TASK.into(), 1, usize::MAX, &viewer)
        .await
        .unwrap();
    let retractions: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.response {
            a2a::StreamResponse::StatusUpdate(u) => u
                .metadata
                .as_ref()?
                .get(TASK_EXT)?
                .get("retracted")
                .cloned(),
            _ => None,
        })
        .collect();
    assert_eq!(
        retractions,
        vec![json!([{ "iteration": 1, "n": 1 }])],
        "exactly one retraction: {events:#?}"
    );
    let failures = recorder.failures.lock().unwrap().clone();
    assert_eq!(
        failures.len(),
        1,
        "one failure, the crashed attempt: {failures:?}"
    );
    assert_eq!(failures[0].0, "call_model");
    assert!(failures[0].1, "the crash surfaces as a timeout");
    assert!(matches!(
        recorder.decisions.lock().unwrap().as_slice(),
        [RetryDecision::Retry { .. }]
    ));

    execution.stop_worker().await.unwrap();
    env.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Scenario: Killing the workflow worker replays without a repeated tool side effect
///     Given an activity-only worker here and a workflow-only worker in a child process
///     When the child is killed right after a side-effecting tool ran
///     Then a workflow worker here replays the history, the tool's recorded result is
///     reused, and the side effect happened exactly once
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chaos_killing_the_workflow_worker_replays_without_a_repeated_tool_side_effect() {
    let (env, address) = dev_server().await;
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("chaos-workflow-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let effects = dir.join("effects.log");
    // The tool reads the effects path from the environment in both processes.
    // SAFETY: set before any thread of this test reads it; the child inherits it.
    unsafe { std::env::set_var(EFFECTS, &effects) };
    let queue = format!("chaos-{}", uuid::Uuid::new_v4());
    let llm: Arc<dyn Llm> = Arc::new(RecordedLlm::new("default", vec![tool_turn(), text("done")]));
    let h = harness(&dir, llm, &address).await;
    let execution = execution(&h, &queue).await;
    execution
        .start_worker_with(WorkerRoles::ActivitiesOnly)
        .unwrap();
    let mut child = spawn_child(&dir, &address, &queue, "workflows", &effects);

    start_task(&h, &execution).await;
    let lines = |path: &Path| std::fs::read_to_string(path).map_or(0, |s| s.lines().count());
    wait_until("the tool to run", Duration::from_secs(60), async || {
        lines(&effects) >= 1
    })
    .await;
    child.kill();

    // Another workflow worker: the history replays; the tool's result is in it.
    execution
        .start_worker_with(WorkerRoles::WorkflowsOnly)
        .unwrap();
    let store = Arc::clone(h.store());
    wait_or_diagnose(
        "the task to complete",
        Duration::from_secs(45),
        &h,
        &execution,
        async || {
            store
                .get_task(&TASK.into())
                .await
                .unwrap()
                .is_some_and(|t| t.is_terminal())
        },
    )
    .await;
    let record = store.get_task(&TASK.into()).await.unwrap().unwrap();
    assert_eq!(record.state, a2a::TaskState::Completed, "{record:?}");
    assert_eq!(
        record.status_message.as_ref().and_then(|m| m.text()),
        Some("done")
    );
    assert_eq!(lines(&effects), 1, "the side effect happened exactly once");

    execution.stop_worker().await.unwrap();
    env.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Scenario: Killing the worker after the peer accepted and before the id was recorded
/// creates no duplicate remote task
///     Given a peer agent served here and an activity-only worker in a child process
///     When the child's `agent.call` has reached the peer (which answers slowly) and the
///     child is killed before the reply
///     Then the retried call reconciles by the intent's context, adopts the peer's task
///     and sends nothing; the peer holds exactly one task for that context (R17.4, R22.4)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn chaos_killing_the_worker_after_the_peer_accepted_and_before_the_id_was_recorded_creates_no_duplicate_remote_task()
 {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn,rusty_harness::harness::a2a_client=debug")
        .with_writer(std::io::stderr)
        .try_init();
    let (env, address) = dev_server().await;
    // The peer: its own harness, served over A2A, answering slowly.
    let peer_llm: Arc<dyn Llm> = Arc::new(SlowLlm {
        id: EntityId::new("default"),
        inner: RecordedLlm::new("default", vec![text("peer says hi"), text("peer says hi")]),
        delay: Duration::from_secs(6),
    });
    let mut peer_config = Config::default();
    peer_config.server.bind = "127.0.0.1:0".into();
    peer_config.agent.name = "peer".into();
    let peer = HarnessBuilder::new(peer_config)
        .host(host(&address))
        .llm(peer_llm)
        .build()
        .await
        .expect("peer builds");
    let peer_server = rusty_harness::app_service::a2a_server::serve(&peer, None)
        .await
        .expect("peer binds");
    let card = format!("http://{}/.well-known/agent-card.json", peer_server.addr);
    tokio::spawn(peer_server.run(std::future::pending()));
    // SAFETY: set before the worker harnesses are built; the child inherits it.
    unsafe { std::env::set_var(PEER_CARD, &card) };

    let dir =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("chaos-peer-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let effects = dir.join("effects.log");
    let queue = format!("chaos-{}", uuid::Uuid::new_v4());
    let llm: Arc<dyn Llm> = Arc::new(RecordedLlm::new("default", vec![text("done")]));
    let h = harness(&dir, llm, &address).await;
    let execution = execution(&h, &queue).await;
    execution
        .start_worker_with(WorkerRoles::WorkflowsOnly)
        .unwrap();
    let mut child = spawn_child(&dir, &address, &queue, "activities", &effects);

    start_task(&h, &execution).await;
    // The peer accepted the child's call: a task exists in the call's own context.
    let peer_store = Arc::clone(peer.store());
    let everyone = rusty_harness::harness::core_loop::persistence::TaskQuery::everyone();
    wait_or_diagnose(
        "the peer to accept the call",
        Duration::from_secs(45),
        &h,
        &execution,
        async || {
            peer_store
                .list_tasks(everyone.clone())
                .await
                .unwrap()
                .items
                .len()
                == 1
        },
    )
    .await;
    // Before the reply (6 s away) and so before the child could record the peer's id.
    child.kill();
    // SAFETY: the child is gone; the environment is this process's again.
    unsafe { std::env::remove_var(PEER_CARD) };
    unsafe { std::env::set_var(PEER_CARD, &card) };

    execution
        .start_worker_with(WorkerRoles::ActivitiesOnly)
        .unwrap();
    let store = Arc::clone(h.store());
    wait_or_diagnose(
        "the task to complete",
        Duration::from_secs(60),
        &h,
        &execution,
        async || {
            store
                .get_task(&TASK.into())
                .await
                .unwrap()
                .is_some_and(|t| t.is_terminal())
        },
    )
    .await;
    let record = store.get_task(&TASK.into()).await.unwrap().unwrap();
    assert_eq!(record.state, a2a::TaskState::Completed, "{record:?}");
    assert_eq!(
        record.status_message.as_ref().and_then(|m| m.text()),
        Some("done")
    );
    let peer_tasks = peer_store.list_tasks(everyone.clone()).await.unwrap().items;
    assert_eq!(
        peer_tasks.len(),
        1,
        "exactly one remote task: {:?}",
        peer_tasks
            .iter()
            .map(|t| (t.id.clone(), t.context_id.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        record.sub_tasks.len(),
        1,
        "one link to it: {:?}",
        record.sub_tasks
    );
    assert_eq!(record.sub_tasks[0].task, peer_tasks[0].id);
    let intent = store
        .get_blob(&format!("outbound/{TASK}/1/call-peer"))
        .await
        .unwrap()
        .expect("the intent");
    let intent: serde_json::Value = serde_json::from_slice(&intent).unwrap();
    assert_eq!(intent["peer_task"], peer_tasks[0].id, "{intent}");

    execution.stop_worker().await.unwrap();
    env.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

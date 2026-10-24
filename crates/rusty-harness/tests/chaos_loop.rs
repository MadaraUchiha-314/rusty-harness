//! Feature: Durable execution, mode `none`
//! Requirement: docs/specs/issue-3/requirements.md#R22
//!
//! T13's `none`-mode half: a child process runs the loop against a file store and is
//! killed mid-task; the parent reopens the store, hands the task to `resume_all`, and the
//! loop continues from the last persisted iteration (R14.4, R22.7).
#![cfg(feature = "rt-tokio")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use rusty_harness::app_service::durable::none::InProcessExecution;
use rusty_harness::app_service::durable::{DurableExecution, RetryPolicy};
use rusty_harness::assets::PromptRegistry;
use rusty_harness::config::Config;
use rusty_harness::harness::commands::{ModelRouter, ModelSelection};
use rusty_harness::harness::core_loop::agent::{
    Agent, AgentRequest, CallContext, LocalAgent, LocalAgentParts,
};
use rusty_harness::harness::core_loop::persistence::{FileStore, TaskQuery};
use rusty_harness::harness::core_loop::task::{Caller, Role, TASK_EXT, TaskRecord};
use rusty_harness::harness::hooks::HookDispatcher;
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{Llm, ModelEvent, StopReason};
use rusty_harness::harness::tools::{
    Idempotency, Tool, ToolCall, ToolCx, ToolDescriptor, ToolError, ToolOutput,
};
use rusty_harness::host::Host;
use rusty_harness::registry::{Entity, EntityId, EntityKind, Registry};
use rusty_harness::runtime::TokioRuntime;
use serde_json::json;

const DIR_VAR: &str = "RH_CHAOS_LOOP_DIR";
const TASK: &str = "chaos-loop";

struct Tick;
impl Entity for Tick {
    fn kind(&self) -> EntityKind {
        EntityKind::Tool
    }
    fn entity_id(&self) -> &EntityId {
        static ID: std::sync::OnceLock<EntityId> = std::sync::OnceLock::new();
        ID.get_or_init(|| EntityId::new("tick"))
    }
}
#[async_trait::async_trait]
impl Tool for Tick {
    fn descriptor(&self) -> &ToolDescriptor {
        static D: std::sync::OnceLock<ToolDescriptor> = std::sync::OnceLock::new();
        D.get_or_init(|| ToolDescriptor {
            name: "tick".into(),
            description: "waits a little".into(),
            parameters: json!({ "type": "object" }),
            idempotency: Idempotency::Idempotent,
        })
    }
    async fn call(&self, _cx: &ToolCx, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        tokio::time::sleep(Duration::from_millis(30)).await;
        Ok(ToolOutput::Value {
            value: json!("tick"),
        })
    }
}

async fn agent(dir: &Path, turns: Vec<Vec<ModelEvent>>) -> Arc<LocalAgent> {
    let registry = Arc::new(Registry::new());
    let llm: Arc<dyn Llm> = Arc::new(RecordedLlm::new("default", turns));
    registry.register(llm).unwrap();
    let tick: Arc<dyn Tool> = Arc::new(Tick);
    registry.register(tick).unwrap();
    let router = Arc::new(ModelRouter::new(
        Arc::clone(&registry),
        ModelSelection {
            model: "default".into(),
            effort: None,
        },
    ));
    let store = Arc::new(FileStore::open(dir).await.unwrap());
    Arc::new(LocalAgent::new(LocalAgentParts {
        config: Config::default(),
        registry,
        dispatcher: Arc::new(HookDispatcher::new()),
        host: Arc::new(Host::testing()),
        store,
        prompts: Arc::new(PromptRegistry::builtin()),
        router,
    }))
}

fn tool_turn(n: usize) -> Vec<ModelEvent> {
    vec![
        ModelEvent::ToolCall {
            call: ToolCall {
                id: format!("c{n}"),
                name: "tick".into(),
                arguments: json!({}),
            },
        },
        ModelEvent::Stop {
            reason: StopReason::ToolUse,
        },
    ]
}

/// The victim: loops on tool calls forever until killed. Run only by the test below.
#[test]
#[ignore = "spawned by chaos_restart_in_none_mode_resumes_from_the_last_persisted_iteration"]
fn chaos_loop_child() {
    let dir = PathBuf::from(std::env::var(DIR_VAR).expect(DIR_VAR));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let agent = agent(&dir, (0..10_000).map(tool_turn).collect()).await;
        let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("keep ticking")]);
        message.metadata = Some(HashMap::from([(
            TASK_EXT.to_owned(),
            json!({ "mode": "long-running" }),
        )]));
        let request = AgentRequest {
            message,
            call: CallContext {
                task_id: TASK.into(),
                context_id: "ctx".into(),
                stored_task: None,
                caller: Caller::human("u1", [Role::Admin]),
                service_params: std::collections::BTreeMap::default(),
            },
            metadata: None,
            extensions: vec![],
        };
        let _ = agent.send_message(request).collect::<Vec<_>>().await;
    });
}

fn persisted_iteration(dir: &Path) -> u32 {
    std::fs::read(dir.join(format!("tasks/{TASK}.json")))
        .ok()
        .and_then(|b| serde_json::from_slice::<TaskRecord>(&b).ok())
        .map_or(0, |t| t.iteration)
}

/// Scenario: A restart in none mode resumes from the last persisted iteration
///     Given a child process looping on tool calls against a file store
///     When it is killed after a few iterations and the store is reopened
///     Then the non-terminal task is resumed and the loop continues from the persisted
///     iteration, keeping the earlier turns, until the model delivers
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn chaos_restart_in_none_mode_resumes_from_the_last_persisted_iteration() {
    let dir =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("chaos-loop-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "chaos_loop_child", "--ignored", "--nocapture"])
        .env(DIR_VAR, &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while persisted_iteration(&dir) < 3 {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the child never iterated"
        );
        if let Some(status) = child.try_wait().unwrap() {
            panic!("the child exited on its own: {status}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.kill().unwrap();
    let _ = child.wait();
    let before = persisted_iteration(&dir);
    assert!(before >= 3);

    let agent = agent(
        &dir,
        vec![vec![
            ModelEvent::TextDelta {
                text: "all ticked".into(),
            },
            ModelEvent::Stop {
                reason: StopReason::EndTurn,
            },
        ]],
    )
    .await;
    let record = agent
        .store()
        .get_task(&TASK.into())
        .await
        .unwrap()
        .expect("the task survived");
    assert_eq!(
        record.state,
        a2a::TaskState::Working,
        "mid-task when killed"
    );
    assert_eq!(record.iteration, before);
    let open = agent
        .store()
        .list_tasks({
            let mut q = TaskQuery::for_viewer(Caller::human("u1", []));
            q.open_only = true;
            q
        })
        .await
        .unwrap();
    assert_eq!(open.items.len(), 1);
    let durable = InProcessExecution::new(
        Arc::clone(&agent),
        Arc::new(TokioRuntime::current()),
        RetryPolicy::default(),
    );
    durable
        .resume_all(open.items.iter().map(|t| t.id.clone()).collect())
        .await
        .unwrap();
    let waited = Instant::now();
    while durable.is_running(&TASK.into())
        || agent
            .store()
            .get_task(&TASK.into())
            .await
            .unwrap()
            .unwrap()
            .state
            == a2a::TaskState::Working
    {
        assert!(
            waited.elapsed() < Duration::from_secs(30),
            "the resumed loop never delivered"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let after = agent.store().get_task(&TASK.into()).await.unwrap().unwrap();
    assert_eq!(
        after.state,
        a2a::TaskState::InputRequired,
        "long-running: delivered and waiting"
    );
    assert_eq!(
        after.iteration,
        before + 1,
        "resumed from the last persisted iteration, one more call"
    );
    let history: Vec<serde_json::Value> = serde_json::from_slice(
        &agent
            .store()
            .get_blob(&format!("history/{TASK}"))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let tool_results = history
        .iter()
        .filter(|t| t["entry"]["kind"] == "tool-result")
        .count();
    assert!(
        tool_results >= before as usize - 1,
        "the earlier turns were kept: {tool_results}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

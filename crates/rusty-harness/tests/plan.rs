//! Feature: Plan and step
//! Requirement: docs/specs/issue-3/requirements.md#R13
//!
//! Scenario: The model plans, records a step, and everyone sees the plan
//!     Given a harness whose model calls `plan.update`, then `plan.step_done`, then answers
//!     When a message is sent
//!     Then the plan streams out as a `plan/v1` artifact, the store holds it with the step
//!     done, and the model's next call sees the plan in its context
//!
//! Scenario: A plan with a cycle is refused
//!     Given a model that sends steps depending on each other in a circle
//!     Then the tool answers the model with an invalid-input error and no plan is stored
#![cfg(feature = "rt-tokio")]

use std::sync::Arc;

use futures::StreamExt as _;
use rusty_harness::HarnessBuilder;
use rusty_harness::config::Config;
use rusty_harness::harness::core_loop::plan::StepStatus;
use rusty_harness::harness::core_loop::plan::tools::{PLAN_MEDIA_TYPE, plan_id};
use rusty_harness::harness::core_loop::task::PLAN_EXT;
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{ModelEvent, StopReason};
use rusty_harness::harness::tools::ToolCall;
use rusty_harness::host::Host;
use serde_json::json;

fn call(id: &str, name: &str, arguments: serde_json::Value) -> Vec<ModelEvent> {
    vec![
        ModelEvent::ToolCall {
            call: ToolCall {
                id: id.into(),
                name: name.into(),
                arguments,
            },
        },
        ModelEvent::Stop {
            reason: StopReason::ToolUse,
        },
    ]
}

fn text(t: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta { text: t.into() },
        ModelEvent::Stop {
            reason: StopReason::EndTurn,
        },
    ]
}

fn request(task: &str) -> a2a::SendMessageRequest {
    let mut message = a2a::Message::new(
        a2a::Role::User,
        vec![a2a::Part::text("Draft the release notes.")],
    );
    message.task_id = Some(task.into());
    a2a::SendMessageRequest {
        message,
        configuration: None,
        metadata: None,
        tenant: None,
    }
}

#[tokio::test]
async fn the_model_plans_records_a_step_and_everyone_sees_the_plan() {
    let steps = json!({ "steps": [
        { "id": "collect", "name": "Collect the changes" },
        { "id": "draft", "name": "Draft the notes", "after": ["collect"] }
    ]});
    let llm = Arc::new(RecordedLlm::new(
        "default",
        vec![
            call("c1", "plan.update", steps),
            call(
                "c2",
                "plan.step_done",
                json!({ "step": "collect", "summary": "12 changes" }),
            ),
            text("Here is the draft."),
        ],
    ));
    let harness = HarnessBuilder::new(Config::default())
        .host(Host::testing())
        .llm(Arc::clone(&llm) as Arc<dyn rusty_harness::harness::models::llm::Llm>)
        .build()
        .await
        .expect("builds");
    let events: Vec<_> = harness.send_message(request("t1"), None).collect().await;

    let plans: Vec<&a2a::Artifact> = events
        .iter()
        .filter_map(|e| match e {
            Ok(a2a::StreamResponse::ArtifactUpdate(u))
                if u.artifact
                    .metadata
                    .as_ref()
                    .is_some_and(|m| m.contains_key(PLAN_EXT)) =>
            {
                Some(&u.artifact)
            }
            _ => None,
        })
        .collect();
    assert_eq!(plans.len(), 2, "one plan artifact per edit: {events:?}");
    assert!(
        plans.iter().all(|a| a.artifact_id == plan_id("t1")),
        "one artifact, replaced"
    );
    assert_eq!(
        plans[0].parts[0].media_type.as_deref(),
        Some(PLAN_MEDIA_TYPE)
    );
    // Read back, the task carries the plan artifact too (a reconnecting renderer sees it).
    let task = harness.get_task("t1", None).await.expect("the task");
    let kept: Vec<_> = task
        .artifacts
        .iter()
        .flatten()
        .filter(|a| a.artifact_id == plan_id("t1"))
        .collect();
    assert_eq!(
        kept.len(),
        1,
        "one plan artifact on the task, the last version: {kept:?}"
    );

    let stored = harness
        .store()
        .get_plan(&plan_id("t1"))
        .await
        .unwrap()
        .expect("the plan is stored");
    assert_eq!(
        stored.step("collect").map(|s| s.status),
        Some(StepStatus::Done)
    );
    assert_eq!(
        stored.step("draft").map(|s| s.status),
        Some(StepStatus::Pending)
    );
    assert_eq!(stored.edges(), [("collect".to_owned(), "draft".to_owned())]);

    let third = &llm.requests()[2];
    let context: String = third
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .filter_map(|p| match p {
            rusty_harness::harness::models::llm::ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        context.contains("- [x] collect: Collect the changes -> 12 changes"),
        "{context}"
    );
    assert!(
        context.contains("draft: Draft the notes (after collect)"),
        "{context}"
    );
}

#[tokio::test]
async fn a_plan_with_a_cycle_is_refused() {
    let circle = json!({ "steps": [
        { "id": "a", "name": "A", "after": ["b"] },
        { "id": "b", "name": "B", "after": ["a"] }
    ]});
    let llm = Arc::new(RecordedLlm::new(
        "default",
        vec![call("c1", "plan.update", circle), text("Understood.")],
    ));
    let harness = HarnessBuilder::new(Config::default())
        .host(Host::testing())
        .llm(Arc::clone(&llm) as Arc<dyn rusty_harness::harness::models::llm::Llm>)
        .build()
        .await
        .expect("builds");
    let _: Vec<_> = harness.send_message(request("t2"), None).collect().await;
    let result = llm.requests()[1]
        .messages
        .iter()
        .filter(|m| m.tool_call_id.as_deref() == Some("c1"))
        .flat_map(|m| m.parts.iter())
        .filter_map(|p| match p {
            rusty_harness::harness::models::llm::ContentPart::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<String>();
    assert!(result.contains("cycle"), "the model is told why: {result}");
    assert!(
        harness
            .store()
            .get_plan(&plan_id("t2"))
            .await
            .unwrap()
            .is_none()
    );
}

/// Scenario: Two steps recorded in one turn are both kept
///     Given a file-backed harness whose model plans three steps
///     When the model records two of them done in one turn (two concurrent tool calls)
///     Then the stored plan has both done: neither read-modify-write overwrites the other
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_steps_recorded_in_one_turn_are_both_kept() {
    let steps = json!({ "steps": [
        { "id": "a", "name": "A" },
        { "id": "b", "name": "B" },
        { "id": "c", "name": "C" }
    ]});
    let both = vec![
        ModelEvent::ToolCall {
            call: ToolCall {
                id: "c2".into(),
                name: "plan.step_done".into(),
                arguments: json!({ "step": "a" }),
            },
        },
        ModelEvent::ToolCall {
            call: ToolCall {
                id: "c3".into(),
                name: "plan.step_done".into(),
                arguments: json!({ "step": "b" }),
            },
        },
        ModelEvent::Stop {
            reason: StopReason::ToolUse,
        },
    ];
    let dir = std::env::temp_dir().join(format!("rh-plan-race-{}", uuid::Uuid::new_v4()));
    let mut config = Config::default();
    config.persistence.store = "file".into();
    config.persistence.path = Some(dir.clone());
    let llm = Arc::new(RecordedLlm::new(
        "default",
        vec![call("c1", "plan.update", steps), both, text("Done.")],
    ));
    let harness = HarnessBuilder::new(config)
        .host(Host::native())
        .llm(Arc::clone(&llm) as Arc<dyn rusty_harness::harness::models::llm::Llm>)
        .build()
        .await
        .expect("builds");
    let _: Vec<_> = harness
        .send_message(request("t-race"), None)
        .collect()
        .await;
    let stored = harness
        .store()
        .get_plan(&plan_id("t-race"))
        .await
        .unwrap()
        .expect("the plan is stored");
    let _ = std::fs::remove_dir_all(dir);
    for step in ["a", "b"] {
        assert_eq!(
            stored.step(step).map(|s| s.status),
            Some(StepStatus::Done),
            "step {step}: {stored:?}"
        );
    }
}

/// Scenario: A vetoed step record is denied, not failed
///     Given a `plan.persist` hook that refuses any plan with a finished step
///     When the model plans and then records a step done
///     Then the model is told the record was denied, with the hook's reason (R4)
#[tokio::test]
async fn a_vetoed_step_record_is_denied_not_failed() {
    use rusty_harness::harness::core_loop::plan::{Plan, PlanPersist};
    use rusty_harness::harness::hooks::{
        Hook, HookCx, HookError, HookOrder, PreHook, PreOutcome, Veto,
    };
    use rusty_harness::registry::EntityId;
    struct Frozen;
    #[async_trait::async_trait]
    impl PreHook<PlanPersist> for Frozen {
        async fn pre(&self, _cx: &HookCx, plan: Plan) -> Result<PreOutcome<Plan>, HookError> {
            if plan.step("a").is_some_and(|s| s.status == StepStatus::Done) {
                Ok(PreOutcome::Veto(Veto {
                    reason: "policy.frozen-plan".into(),
                    detail: None,
                    by: EntityId::new("frozen"),
                }))
            } else {
                Ok(PreOutcome::Continue(plan))
            }
        }
    }
    let llm = Arc::new(RecordedLlm::new(
        "default",
        vec![
            call(
                "c1",
                "plan.update",
                json!({ "steps": [{ "id": "a", "name": "A" }] }),
            ),
            call("c2", "plan.step_done", json!({ "step": "a" })),
            text("Noted."),
        ],
    ));
    let harness = HarnessBuilder::new(Config::default())
        .host(Host::testing())
        .llm(Arc::clone(&llm) as Arc<dyn rusty_harness::harness::models::llm::Llm>)
        .build()
        .await
        .expect("builds");
    harness.hooks().register::<PlanPersist>(
        EntityId::new("frozen"),
        Hook::Pre(Arc::new(Frozen)),
        HookOrder::default(),
    );
    let _: Vec<_> = harness
        .send_message(request("t-veto"), None)
        .collect()
        .await;
    let told = llm.requests()[2]
        .messages
        .iter()
        .filter(|m| m.tool_call_id.as_deref() == Some("c2"))
        .flat_map(|m| m.parts.iter())
        .filter_map(|p| match p {
            rusty_harness::harness::models::llm::ContentPart::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<String>();
    assert!(
        told.contains("policy.frozen-plan"),
        "the reason reaches the model: {told}"
    );
    assert!(told.contains("denied"), "a veto is a denial: {told}");
    assert!(!told.contains("failed"), "not an internal failure: {told}");
}

/// Scenario: A plan hook sees the harness's registry
///     Given a `plan.persist` hook that looks up the registered tools
///     When the model sets a plan
///     Then the hook finds the built-in tools, `plan.update` among them, as every other
///     operation's hooks do
#[tokio::test]
async fn a_plan_hook_sees_the_harness_registry() {
    use rusty_harness::harness::core_loop::plan::{Plan, PlanPersist};
    use rusty_harness::harness::hooks::{Hook, HookCx, HookError, HookOrder, PreHook, PreOutcome};
    use rusty_harness::harness::tools::Tool;
    use rusty_harness::registry::EntityId;
    struct Looks(Arc<std::sync::Mutex<Vec<String>>>);
    #[async_trait::async_trait]
    impl PreHook<PlanPersist> for Looks {
        async fn pre(&self, cx: &HookCx, plan: Plan) -> Result<PreOutcome<Plan>, HookError> {
            let names = cx
                .registry
                .list::<dyn Tool>()
                .iter()
                .map(|t| t.entity_id().id.to_string())
                .collect();
            *self.0.lock().unwrap() = names;
            Ok(PreOutcome::Continue(plan))
        }
    }
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let llm = Arc::new(RecordedLlm::new(
        "default",
        vec![
            call(
                "c1",
                "plan.update",
                json!({ "steps": [{ "id": "a", "name": "A" }] }),
            ),
            text("Planned."),
        ],
    ));
    let harness = HarnessBuilder::new(Config::default())
        .host(Host::testing())
        .llm(Arc::clone(&llm) as Arc<dyn rusty_harness::harness::models::llm::Llm>)
        .build()
        .await
        .expect("builds");
    harness.hooks().register::<PlanPersist>(
        EntityId::new("looks"),
        Hook::Pre(Arc::new(Looks(Arc::clone(&seen)))),
        HookOrder::default(),
    );
    let _: Vec<_> = harness.send_message(request("t-reg"), None).collect().await;
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.iter().any(|n| n == "plan.update"),
        "the hook saw: {seen:?}"
    );
}

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

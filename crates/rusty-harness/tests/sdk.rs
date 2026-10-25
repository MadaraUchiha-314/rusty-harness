//! Feature: Packaging and SDK parity
//! Requirement: docs/specs/issue-3/requirements.md#R30
//!
//! Scenario: The harness is usable from Rust in process
//!     Given a config, a testing host and a scripted model handed to the builder
//!     When a message is sent through the built agent
//!     Then the loop runs with no service layer and the reply is streamed back,
//!     And the built-ins (tools, commands, loopback agent) are registered

use std::sync::Arc;

use futures::StreamExt as _;
use rusty_harness::config::Config;
use rusty_harness::harness::core_loop::agent::{Agent, AgentRequest, CallContext};
use rusty_harness::harness::core_loop::task::{Caller, Role};
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{ModelEvent, StopReason};
use rusty_harness::harness::tools::Tool;
use rusty_harness::host::Host;
use rusty_harness::{HarnessBuilder, SdkError};

fn scripted() -> Arc<RecordedLlm> {
    Arc::new(RecordedLlm::new(
        "default",
        vec![vec![
            ModelEvent::TextDelta {
                text: "Hello from the loop.".into(),
            },
            ModelEvent::Stop {
                reason: StopReason::EndTurn,
            },
        ]],
    ))
}

#[tokio::test]
async fn the_harness_is_usable_from_rust_in_process() {
    let harness = HarnessBuilder::new(Config::default())
        .host(Host::testing())
        .llm(scripted())
        .build()
        .await
        .expect("builds");
    let tools: Vec<String> = harness
        .registry()
        .list::<dyn Tool>()
        .iter()
        .map(|t| t.descriptor().name.clone())
        .collect();
    for built_in in ["agent.call", "participant.ask", "participant.delegate"] {
        assert!(
            tools.iter().any(|t| t == built_in),
            "{built_in} in {tools:?}"
        );
    }
    assert_eq!(
        harness.registry().list::<dyn Agent>().len(),
        1,
        "the loopback agent"
    );
    assert_eq!(harness.agent().card().name, "rusty-harness");

    let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("say hello")]);
    message.task_id = Some("t1".into());
    let request = AgentRequest {
        message,
        call: CallContext {
            task_id: "t1".into(),
            context_id: "ctx".into(),
            stored_task: None,
            caller: Caller::human("me", [Role::Admin]),
            service_params: std::collections::BTreeMap::default(),
        },
        metadata: None,
        extensions: vec![],
        inbox_event_id: None,
        traceparent: None,
    };
    let events: Vec<_> = harness.agent().send_message(request).collect().await;
    assert!(
        events.iter().any(|e| matches!(e, Ok(a2a::StreamResponse::Message(m)) if m.text() == Some("Hello from the loop."))),
        "{events:?}"
    );
    let task = harness
        .store()
        .get_task(&"t1".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.state, a2a::TaskState::Completed, "one-shot by default");
    assert!(
        harness.durable().is_some(),
        "the in-process durable mode is shared"
    );
    assert!(harness.inbox().is_some());
}

#[tokio::test]
async fn building_fails_closed_without_a_default_model_or_with_a_bad_store() {
    let err = HarnessBuilder::new(Config::default())
        .host(Host::testing())
        .build()
        .await
        .expect_err("fails");
    assert!(matches!(err, SdkError::NoDefaultModel), "{err}");
    let mut config = Config::default();
    config.persistence.store = "file".into();
    let err = HarnessBuilder::new(config)
        .host(Host::testing())
        .llm(scripted())
        .build()
        .await
        .expect_err("fails");
    assert!(matches!(err, SdkError::Store(_)), "{err}");
}

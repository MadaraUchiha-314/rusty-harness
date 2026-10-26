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

/// Scenario: A `file:` system prompt replaces the default (R7.2)
///     Given `[agent] system_prompt = "file:prompts/mine.md"` and that file on the host
///     When a message is sent
///     Then the model is instructed with the file's body, not the default prompt
#[tokio::test]
async fn a_file_system_prompt_replaces_the_default() {
    use rusty_harness::host::testing::{MemFs, MockHttp};
    let file = "---\nid: prompts/mine\nversion: 1.0.0\nplaceholders: []\ndescription: Mine.\n---\nYou are the replacement prompt.\n";
    let host = Host::builder()
        .http(Arc::new(MockHttp::always(
            http::StatusCode::NOT_FOUND,
            bytes::Bytes::new(),
        )))
        .clock(Arc::new(rusty_harness::host::testing::FixedClock::default()))
        .rng(Arc::new(rusty_harness::host::testing::SeededRng::new(7)))
        .secrets(Arc::new(rusty_harness::host::testing::MapSecrets::default()))
        .fs(Arc::new(MemFs::new([(
            std::path::PathBuf::from("prompts/mine.md"),
            bytes::Bytes::from_static(file.as_bytes()),
        )])))
        .build();
    let mut config = Config::default();
    config.agent.system_prompt = Some("file:prompts/mine.md".into());
    let llm = scripted();
    let harness = HarnessBuilder::new(config)
        .host(host)
        .llm(Arc::clone(&llm) as Arc<dyn rusty_harness::harness::models::llm::Llm>)
        .build()
        .await
        .expect("builds with the replacement");
    let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("hi")]);
    message.task_id = Some("t-file".into());
    let request = a2a::SendMessageRequest {
        message,
        configuration: None,
        metadata: None,
        tenant: None,
    };
    let _: Vec<_> = harness.send_message(request, None).collect().await;
    let instructions = llm.requests()[0].instructions.render();
    assert!(
        instructions.contains("You are the replacement prompt."),
        "{instructions}"
    );
    assert!(
        !instructions.contains("You work on one task at a time"),
        "the default is replaced"
    );

    let mut missing = Config::default();
    missing.agent.system_prompt = Some("file:prompts/absent.md".into());
    let err = HarnessBuilder::new(missing)
        .host(Host::testing())
        .llm(scripted())
        .build()
        .await
        .expect_err("fails closed");
    assert!(matches!(err, SdkError::Prompt(_)), "{err}");
}

/// Scenario: A person's command runs instead of the model (R10.1, R10.6)
///     Given a harness whose model must not be called
///     When `/model default` and then `/modle` are sent
///     Then each is answered with its line and a `commands/v1` outcome, and the model is
///     never called
#[tokio::test]
async fn a_persons_command_runs_instead_of_the_model() {
    use rusty_harness::harness::commands::{COMMANDS_EXT, CommandsMeta};
    let llm = Arc::new(RecordedLlm::new("default", vec![]));
    let harness = HarnessBuilder::new(Config::default())
        .host(Host::testing())
        .llm(Arc::clone(&llm) as Arc<dyn rusty_harness::harness::models::llm::Llm>)
        .build()
        .await
        .expect("builds");
    let send = |text: &str| {
        let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text(text)]);
        message.task_id = Some(format!("t-{}", text.len()));
        a2a::SendMessageRequest {
            message,
            configuration: None,
            metadata: None,
            tenant: None,
        }
    };
    for (text, outcome, says) in [
        ("/model default", "done", "default"),
        ("/modle", "unknown", "/model"),
    ] {
        let events: Vec<_> = harness.send_message(send(text), None).collect().await;
        let [Ok(a2a::StreamResponse::Message(reply))] = events.as_slice() else {
            panic!("one reply message for {text}: {events:?}");
        };
        let meta: CommandsMeta =
            serde_json::from_value(reply.metadata.as_ref().unwrap()[COMMANDS_EXT].clone()).unwrap();
        assert_eq!(
            meta.result.as_ref().map(|r| r.outcome.as_str()),
            Some(outcome),
            "{text}"
        );
        assert_eq!(
            meta.tokens.first().map(|t| (t.start, t.end)),
            Some((
                0,
                text.len()
                    .min(1 + text[1..].find(' ').unwrap_or(text.len() - 1))
            )),
            "{text}"
        );
        assert!(
            reply.text().is_some_and(|t| t.contains(says)),
            "{text}: {reply:?}"
        );
    }
    assert_eq!(llm.attempts(), 0, "no command reached the model");
}

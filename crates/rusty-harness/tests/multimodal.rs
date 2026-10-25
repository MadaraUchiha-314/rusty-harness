//! Feature: Multi-modal input and output
//! Requirement: docs/specs/issue-3/requirements.md#R28
//!
//! The task 38 scenarios: an image a text-only model cannot take is refused before any
//! task exists, naming the modality (R28.1); an image a vision model can take reaches it
//! as a content part (R28.1); an image a tool renders reaches the participants as an
//! artifact (R28.2).
#![cfg(feature = "server")]

use std::collections::BTreeSet;
use std::sync::Arc;

use rusty_harness::app_service::a2a_server::serve;
use rusty_harness::config::Config;
use rusty_harness::harness::core_loop::task::{Caller, TASK_EXT};
use rusty_harness::harness::models::llm::testing::RecordedLlm;
use rusty_harness::harness::models::llm::{
    ContentPart, Effort, Modality, ModelCapabilities, ModelEvent, StopReason,
};
use rusty_harness::harness::tools::{
    Idempotency, Tool, ToolCall, ToolCx, ToolDescriptor, ToolError, ToolOutput,
};
use rusty_harness::host::Host;
use rusty_harness::registry::{Entity, EntityId, EntityKind};
use rusty_harness::{Harness, HarnessBuilder, harness_async_trait};
use serde_json::{Value, json};

/// The first eight bytes of a PNG, base64: enough to be an image part.
const PNG_B64: &str = "iVBORw0KGgo=";

fn text(t: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta { text: t.into() },
        ModelEvent::Stop {
            reason: StopReason::EndTurn,
        },
    ]
}

fn caps(modalities: &[Modality]) -> ModelCapabilities {
    ModelCapabilities {
        input_modalities: modalities.iter().copied().collect::<BTreeSet<_>>(),
        tools: true,
        caching: false,
        max_window_tokens: 128_000,
        effort_levels: Vec::<Effort>::new(),
    }
}

async fn harness(llm: Arc<RecordedLlm>, tool: Option<Arc<dyn Tool>>) -> Harness {
    let mut config = Config::default();
    config.server.bind = "127.0.0.1:0".into();
    let mut builder = HarnessBuilder::new(config).host(Host::testing()).llm(llm);
    if let Some(tool) = tool {
        builder = builder.tool(tool);
    }
    builder.build().await.expect("builds")
}

async fn send(h: &Harness, parts: Value) -> Value {
    let server = serve(h, None).await.expect("binds");
    let base = format!("http://{}/", server.addr);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(server.run(async move {
        let _ = stopped.await;
    }));
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "SendMessage",
        "params": { "message": {
            "messageId": uuid::Uuid::new_v4().to_string(),
            "role": "ROLE_USER",
            "parts": parts,
            "metadata": { TASK_EXT: { "mode": "long-running" } }
        } }
    });
    let reply: Value = reqwest::Client::new()
        .post(&base)
        .header("A2A-Version", "1.0")
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let _ = stop.send(());
    let _ = task.await;
    reply
}

/// Scenario: An image sent to a text-only model is rejected naming the modality
///     Given a harness whose model accepts text only
///     When a participant sends a message with an image part
///     Then the answer is `ContentTypeNotSupported` naming the image modality
///     And no task was created and the model was never called
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_image_sent_to_a_text_only_model_is_rejected_naming_the_modality() {
    let llm = Arc::new(
        RecordedLlm::new("default", vec![text("never")]).with_capabilities(caps(&[Modality::Text])),
    );
    let h = harness(Arc::clone(&llm), None).await;
    let reply = send(
        &h,
        json!([{ "text": "what is in this picture?" }, { "raw": PNG_B64, "mediaType": "image/png" }]),
    )
    .await;
    assert_eq!(reply["error"]["code"], -32005, "{reply}");
    let message = reply["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("Image"), "names the modality: {message}");
    assert!(message.contains("default"), "names the model: {message}");
    assert!(llm.requests().is_empty(), "the model was never called");
}

/// Scenario: An image sent to a vision model reaches it as an image part
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_image_sent_to_a_vision_model_reaches_it_as_an_image_part() {
    let llm = Arc::new(
        RecordedLlm::new("default", vec![text("a cat")])
            .with_capabilities(caps(&[Modality::Text, Modality::Image])),
    );
    let h = harness(Arc::clone(&llm), None).await;
    let reply = send(
        &h,
        json!([{ "text": "what is in this picture?" }, { "raw": PNG_B64, "mediaType": "image/png", "filename": "cat.png" }]),
    )
    .await;
    assert_eq!(
        reply["result"]["task"]["status"]["state"], "TASK_STATE_INPUT_REQUIRED",
        "{reply}"
    );
    let requests = llm.requests();
    let parts: Vec<&ContentPart> = requests[0]
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .collect();
    assert!(
        parts.iter().any(|p| matches!(p, ContentPart::Image { media_type, data } if media_type == "image/png" && data.starts_with(b"\x89PNG"))),
        "{parts:?}"
    );
}

/// A tool that renders a chart: a value for the model, a PNG for the participants.
struct Chart(EntityId, ToolDescriptor);

impl Entity for Chart {
    fn kind(&self) -> EntityKind {
        EntityKind::Tool
    }
    fn entity_id(&self) -> &EntityId {
        &self.0
    }
}

harness_async_trait! {
    impl Tool for Chart {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.1
        }
        async fn call(&self, _cx: &ToolCx, _input: Value) -> Result<ToolOutput, ToolError> {
            let mut part = a2a::Part::data(Value::Null).with_media_type("image/png");
            part.content = a2a::PartContent::Raw(b"\x89PNG\r\n\x1a\n".to_vec());
            Ok(ToolOutput::Artifacts {
                value: json!({ "rendered": true }),
                artifacts: vec![a2a::Artifact {
                    artifact_id: "chart-1".into(),
                    name: Some("chart.png".into()),
                    description: Some("revenue by quarter".into()),
                    parts: vec![part],
                    metadata: None,
                    extensions: None,
                }],
            })
        }
    }
}

/// Scenario: An image a tool renders reaches the participants as an artifact
///     Given a tool that returns a PNG artifact with its value
///     When the model calls it
///     Then the task's log holds the artifact update and the model learns its name only
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_image_a_tool_renders_reaches_the_participants_as_an_artifact() {
    let call = vec![
        ModelEvent::ToolCall {
            call: ToolCall {
                id: "c1".into(),
                name: "chart.render".into(),
                arguments: json!({}),
            },
        },
        ModelEvent::Stop {
            reason: StopReason::ToolUse,
        },
    ];
    let llm = Arc::new(RecordedLlm::new(
        "default",
        vec![call, text("Here is the chart.")],
    ));
    let tool: Arc<dyn Tool> = Arc::new(Chart(
        EntityId::new("chart.render"),
        ToolDescriptor {
            name: "chart.render".into(),
            description: "Renders a chart.".into(),
            parameters: json!({ "type": "object" }),
            idempotency: Idempotency::Idempotent,
        },
    ));
    let h = harness(Arc::clone(&llm), Some(tool)).await;
    let reply = send(&h, json!([{ "text": "chart the revenue" }])).await;
    let task = reply["result"]["task"]["id"]
        .as_str()
        .expect("a task")
        .to_owned();
    let events = h
        .store()
        .events(&task, 1, usize::MAX, &Caller::system("test"))
        .await
        .unwrap();
    let artifact = events
        .iter()
        .find_map(|e| match &e.response {
            a2a::StreamResponse::ArtifactUpdate(u) => Some(u.artifact.clone()),
            _ => None,
        })
        .expect("an artifact update in the log");
    assert_eq!(artifact.name.as_deref(), Some("chart.png"));
    assert_eq!(artifact.parts[0].media_type.as_deref(), Some("image/png"));
    let second = &llm.requests()[1];
    let told: String = second
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        told.contains("[artifact chart.png (image/png) delivered to the participants]"),
        "{told}"
    );
    assert!(
        !told.contains("PNG\\r\\n"),
        "the bytes never reach the model"
    );
}

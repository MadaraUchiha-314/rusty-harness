//! Feature: A2A extensions
//! Requirement: docs/specs/issue-3/requirements.md#R19
//!
//! Scenario: The six extension schemas match the checked-in files
//!     Given the payload types of task/v1, participants/v1, plan/v1, channels/v1,
//!     commands/v1 and voice/v1
//!     When their JSON schemas are generated
//!     Then each equals `specs/a2a-ext/<name>.v1.schema.json` (`UPDATE_SNAPSHOTS=1` rewrites)

use std::path::Path;

use rusty_harness::harness::core_loop::task::ext;

#[test]
fn contract_a2a_extension_schemas() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../specs/a2a-ext");
    let all = ext::all();
    assert_eq!(all.len(), 6);
    let mut stale = Vec::new();
    for extension in &all {
        assert_eq!(
            extension.uri,
            format!(
                "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/{}/v1",
                extension.name
            )
        );
        let path = dir.join(format!("{}.v1.schema.json", extension.name));
        let mut json = serde_json::to_string_pretty(&extension.schema).expect("serializes");
        json.push('\n');
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &json).expect("writes the schema");
        }
        let published: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default())
                .unwrap_or(serde_json::Value::Null);
        let generated: serde_json::Value = serde_json::from_str(&json).unwrap();
        if published != generated {
            stale.push(path.display().to_string());
        }
        // The docs site serves the same schema next to the extension's page (Q12).
        let served = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/public/a2a/ext")
            .join(extension.name)
            .join("v1.schema.json");
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            std::fs::create_dir_all(served.parent().unwrap()).unwrap();
            std::fs::write(&served, &json).expect("writes the served schema");
        }
        let on_site: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&served).unwrap_or_default())
                .unwrap_or(serde_json::Value::Null);
        if on_site != generated {
            stale.push(served.display().to_string());
        }
    }
    assert!(
        stale.is_empty(),
        "stale extension schemas, rerun with UPDATE_SNAPSHOTS=1: {stale:#?}"
    );
    let uris = ext::uris();
    assert!(
        uris.iter()
            .all(|u| u.starts_with(rusty_harness::harness::core_loop::task::EXT_PREFIX))
    );
}

/// One payload per extension, as the harness itself produces them: a turn that plans and
/// asks through A2UI (task, participants, plan, channels), the card (commands), a voice
/// audio part (voice).
#[cfg(feature = "rt-tokio")]
#[allow(clippy::too_many_lines)] // one producer per extension, read top to bottom
async fn produced_payloads() -> std::collections::BTreeMap<&'static str, serde_json::Value> {
    use futures::StreamExt as _;
    use rusty_harness::harness::channels::CHANNELS_EXT;
    use rusty_harness::harness::commands::COMMANDS_EXT;
    use rusty_harness::harness::core_loop::task::{Attempt, PARTICIPANTS_EXT, PLAN_EXT, TASK_EXT};
    use rusty_harness::harness::models::llm::testing::RecordedLlm;
    use rusty_harness::harness::models::llm::{ModelEvent, StopReason};
    use rusty_harness::harness::models::speech::{AudioChunk, VOICE_EXT, VoiceMeta, audio_part};
    use rusty_harness::harness::tools::ToolCall;
    use rusty_harness::host::{AudioFormat, Host};
    use serde_json::json;

    let call = |id: &str, name: &str, arguments: serde_json::Value| {
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
    };
    let form = json!([
        { "version": "v0.9.1", "createSurface": { "surfaceId": "ask", "catalogId": "https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json" } },
        { "version": "v0.9.1", "updateComponents": { "surfaceId": "ask", "components": [{ "id": "root", "component": "Text", "text": "Which audience?" }] } }
    ]);
    let llm = std::sync::Arc::new(RecordedLlm::new(
        "default",
        vec![
            call(
                "c1",
                "plan.update",
                json!({ "steps": [{ "id": "draft", "name": "Draft the notes" }] }),
            ),
            call(
                "c2",
                "participant.ask",
                json!({ "to": ["operator"], "kind": "a2ui", "question": form }),
            ),
        ],
    ));
    let harness = rusty_harness::HarnessBuilder::new(rusty_harness::config::Config::default())
        .host(Host::testing())
        .llm(llm)
        .build()
        .await
        .expect("builds");
    let mut message = a2a::Message::new(
        a2a::Role::User,
        vec![a2a::Part::text("Draft the release notes.")],
    );
    message.task_id = Some("t-ext".into());
    message.metadata = Some([(TASK_EXT.to_owned(), json!({ "mode": "long-running" }))].into());
    let request = a2a::SendMessageRequest {
        message,
        configuration: None,
        metadata: None,
        tenant: None,
    };
    let _: Vec<_> = harness.send_message(request, None).collect().await;
    let task = harness.get_task("t-ext", None).await.expect("the task");
    let meta = task.metadata.clone().unwrap_or_default();
    let status = task
        .status
        .message
        .as_ref()
        .and_then(|m| m.metadata.clone())
        .unwrap_or_default();
    let plan = task
        .artifacts
        .iter()
        .flatten()
        .find_map(|a| a.metadata.as_ref().and_then(|m| m.get(PLAN_EXT)).cloned());
    // commands/v1 on the wire: the reply to a command carries its tokens and outcome.
    let mut command = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("/model default")]);
    command.task_id = Some("t-cmd".into());
    let request = a2a::SendMessageRequest {
        message: command,
        configuration: None,
        metadata: None,
        tenant: None,
    };
    let replies: Vec<_> = harness.send_message(request, None).collect().await;
    let commands = replies.iter().find_map(|e| match e {
        Ok(a2a::StreamResponse::Message(m)) if m.role == a2a::Role::Agent => m
            .metadata
            .as_ref()
            .and_then(|md| md.get(COMMANDS_EXT))
            .cloned(),
        Ok(a2a::StreamResponse::StatusUpdate(u)) => u
            .status
            .message
            .as_ref()
            .and_then(|m| m.metadata.as_ref())
            .and_then(|md| md.get(COMMANDS_EXT))
            .cloned(),
        _ => None,
    });
    let voice = audio_part(
        &AudioChunk {
            format: AudioFormat::Pcm16 {
                sample_rate: 24_000,
                channels: 1,
            },
            data: bytes::Bytes::from_static(&[0, 0]),
            seq: 0,
        },
        &VoiceMeta {
            seq: 0,
            attempt: Attempt { iteration: 1, n: 1 },
            format: "pcm16@24k".into(),
            is_final: true,
            session: "s1".into(),
        },
    )
    .metadata
    .and_then(|m| m.get(VOICE_EXT).cloned());

    let mut out = std::collections::BTreeMap::new();
    for (name, value) in [
        ("task", meta.get(TASK_EXT).cloned()),
        ("participants", meta.get(PARTICIPANTS_EXT).cloned()),
        ("plan", plan),
        ("channels", status.get(CHANNELS_EXT).cloned()),
        ("commands", commands),
        ("voice", voice),
    ] {
        out.insert(
            name,
            value.unwrap_or_else(|| panic!("the harness produced no {name}/v1 payload")),
        );
    }
    out
}

/// Ids and times vary per run; the shapes and values around them must not.
fn normalize(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            let uuid = s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4;
            let time = chrono::DateTime::parse_from_rfc3339(s).is_ok();
            if uuid {
                *s = "<uuid>".into();
            } else if time {
                *s = "<time>".into();
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(normalize),
        serde_json::Value::Object(map) => map.values_mut().for_each(normalize),
        _ => {}
    }
}

/// Scenario: Every extension payload the harness produces matches its published schema
///     Given one payload per extension, produced by the harness (not written by hand)
///     Then each validates against `specs/a2a-ext/<name>.v1.schema.json`
#[cfg(feature = "rt-tokio")]
#[tokio::test]
async fn contract_extension_payloads_match_their_schemas() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../specs/a2a-ext");
    for (name, payload) in produced_payloads().await {
        let schema: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join(format!("{name}.v1.schema.json")))
                .expect("the schema"),
        )
        .expect("JSON");
        let validator = jsonschema::validator_for(&schema).expect("a valid schema");
        let errors: Vec<String> = validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{name}/v1: {errors:?}\n{payload:#}");
    }
}

/// T6: the payloads themselves, ids and times normalized.
#[cfg(feature = "rt-tokio")]
#[tokio::test]
async fn snapshot_extension_payloads() {
    let mut all = serde_json::Map::new();
    for (name, mut payload) in produced_payloads().await {
        normalize(&mut payload);
        all.insert(name.to_owned(), payload);
    }
    let mut text = serde_json::to_string_pretty(&all).unwrap();
    text.push('\n');
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/extension_payloads.json");
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let published: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default())
            .unwrap_or_default();
    let produced: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        published, produced,
        "extension_payloads.json is stale: rerun with UPDATE_SNAPSHOTS=1"
    );
}

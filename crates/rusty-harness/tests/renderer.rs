//! Feature: Surfaces, renderers and the A2UI model
//! Requirement: docs/specs/issue-3/requirements.md#R25
//!
//! The task 33 snapshot: the status message of an A2UI ask as the harness emits it — the
//! text alternative, the `application/a2ui+json` data part and the `channels/v1` and
//! A2UI extension metadata (T6, `snapshot_a2ui_ask_payload`).

use std::collections::BTreeSet;

use rusty_harness::harness::channels::{Ask, Channel, QuestionKind, question_message};
use rusty_harness::harness::commands::ModelSelection;
use rusty_harness::harness::core_loop::task::{
    Participant, ParticipantKind, Role, TaskDraft, TaskMode, TaskRecord,
};
use rusty_harness::interaction::renderer::a2ui::{
    A2uiMessage, Action, ActionEvent, BASIC_CATALOG, ButtonVariant, Children, ChoiceOption,
    ChoiceVariant, Component, ComponentKind, CreateSurface, Dynamic, EventAction, TextVariant,
    UpdateComponents,
};

fn ids(ids: &[&str]) -> Children {
    Children::Ids(ids.iter().map(|s| (*s).to_owned()).collect())
}

/// The ask of the TUI prototype (`design/tui-renderer.html`, state `a2ui`).
#[allow(clippy::too_many_lines)]
fn audience_form() -> Vec<A2uiMessage> {
    let components = vec![
        Component::new(
            "root",
            ComponentKind::Column {
                children: ids(&["question", "detail", "audience", "other", "actions"]),
                justify: None,
                align: None,
            },
        ),
        Component::new(
            "question",
            ComponentKind::Text {
                text: "Which audience are these notes for?".into(),
                variant: Some(TextVariant::H3),
            },
        ),
        Component::new(
            "detail",
            ComponentKind::Text {
                text: "The tone and the level of detail depend on it. Pick one, or type another."
                    .into(),
                variant: Some(TextVariant::Body),
            },
        ),
        Component::new(
            "audience",
            ComponentKind::ChoicePicker {
                label: None,
                variant: Some(ChoiceVariant::MutuallyExclusive),
                options: vec![
                    ChoiceOption::new("Customers", "customers"),
                    ChoiceOption::new("Internal engineering", "internal"),
                    ChoiceOption::new("Both, two sections", "both"),
                ],
                value: Dynamic::path("/audience"),
                display_style: None,
                filterable: None,
                checks: None,
            },
        ),
        Component::new(
            "other",
            ComponentKind::TextField {
                label: "Something else".into(),
                value: Some(Dynamic::path("/other")),
                variant: None,
                validation_regexp: None,
                checks: None,
            },
        ),
        Component::new(
            "actions",
            ComponentKind::Row {
                children: ids(&["send", "skip"]),
                justify: None,
                align: None,
            },
        ),
        Component::new(
            "send",
            ComponentKind::Button {
                child: "send_label".into(),
                variant: Some(ButtonVariant::Primary),
                action: Action::Event(EventAction {
                    event: ActionEvent {
                        name: "answer".into(),
                        context: Some(
                            [
                                ("audience".to_owned(), Dynamic::path("/audience")),
                                ("other".to_owned(), Dynamic::path("/other")),
                            ]
                            .into_iter()
                            .collect(),
                        ),
                    },
                }),
                checks: None,
            },
        ),
        Component::new(
            "send_label",
            ComponentKind::Text {
                text: "Send answer".into(),
                variant: None,
            },
        ),
        Component::new(
            "skip",
            ComponentKind::Button {
                child: "skip_label".into(),
                variant: Some(ButtonVariant::Borderless),
                action: Action::event("skip"),
                checks: None,
            },
        ),
        Component::new(
            "skip_label",
            ComponentKind::Text {
                text: "Skip".into(),
                variant: None,
            },
        ),
    ];
    vec![
        A2uiMessage::CreateSurface(CreateSurface {
            surface_id: "ask-ch-7f3a".into(),
            catalog_id: BASIC_CATALOG.into(),
            theme: None,
            send_data_model: Some(true),
        }),
        A2uiMessage::UpdateComponents(UpdateComponents {
            surface_id: "ask-ch-7f3a".into(),
            components,
        }),
    ]
}

/// T6: the A2UI ask as the status message carries it, snapshotted.
#[test]
fn snapshot_a2ui_ask_payload() {
    let record = TaskRecord::new(
        TaskDraft {
            id: "task_01HZX4V3K9N2".into(),
            context_id: "ctx_9a1f".into(),
            name: "release notes".into(),
            r#type: None,
            goal: "write the release notes".into(),
            description: String::new(),
            acceptance_criteria: vec![],
            mode: TaskMode::LongRunning,
            participants: vec![
                Participant::human("you", "You", [Role::Reporter, Role::Admin]),
                Participant {
                    id: "demo-agent".into(),
                    display: "demo-agent".into(),
                    kind: ParticipantKind::System,
                    roles: BTreeSet::from([Role::Assignee]),
                },
            ],
            parent_tasks: vec![],
            reference_task_ids: vec![],
            model: ModelSelection {
                model: "default".into(),
                effort: None,
            },
        },
        chrono::DateTime::parse_from_rfc3339("2026-10-24T12:33:00Z")
            .unwrap()
            .into(),
    );
    let channel = Channel {
        id: "ch-7f3a".into(),
        task: record.id.clone(),
        members: BTreeSet::from(["you".to_owned()]),
        created_by: "demo-agent".into(),
        open: true,
    };
    let ask = Ask {
        task: record.id.clone(),
        call_id: "call_1".into(),
        to: vec!["you".into()],
        question: serde_json::to_value(audience_form()).unwrap(),
        kind: QuestionKind::A2ui,
        text: None,
        timeout_s: None,
    };
    let mut message = question_message(&record, &channel, &ask);
    message.message_id = "msg_ask".into();

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots/a2ui_ask_payload.json");
    let mut json = serde_json::to_string_pretty(&serde_json::to_value(&message).unwrap()).unwrap();
    json.push('\n');
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &json).unwrap();
    }
    let published: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default())
            .unwrap_or(serde_json::Value::Null);
    assert_eq!(
        published,
        serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        "a2ui_ask_payload.json is stale: rerun with UPDATE_SNAPSHOTS=1"
    );
    // The shape the extension prescribes: the text alternative first, then the data part
    // whose data is the message list and whose metadata names the media type.
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        value["parts"][0]["text"],
        "Which audience are these notes for?\nThe tone and the level of detail depend on it. Pick one, or type another.\nCustomers / Internal engineering / Both, two sections\nSomething else\nSend answer · Skip"
    );
    assert_eq!(
        value["parts"][0]["metadata"]["alternativeFor"],
        "application/a2ui+json"
    );
    assert_eq!(
        value["parts"][1]["metadata"]["mimeType"],
        "application/a2ui+json"
    );
    assert_eq!(value["parts"][1]["data"][0]["version"], "v0.9.1");
    assert_eq!(
        value["parts"][1]["data"][0]["createSurface"]["catalogId"],
        BASIC_CATALOG
    );
    assert_eq!(
        value["parts"][1]["data"][1]["updateComponents"]["components"][0]["id"],
        "root"
    );
}

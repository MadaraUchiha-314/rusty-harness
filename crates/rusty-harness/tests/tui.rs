//! Feature: The terminal renderer
//! Requirement: docs/specs/issue-3/requirements.md#R27
//!
//! The task 34 checks: one `TestBackend` buffer per prototype state (T5,
//! `tui_snapshot_*`: chat, a2ui, model, done, first), the `NO_COLOR` leg of each (T9),
//! the image artifact scenario (R28.2) and the A2UI form's reply (R18, R25.4).
#![cfg(feature = "tui")]

use std::collections::HashMap;

use chrono::{DateTime, TimeZone, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use rusty_harness::harness::channels::CHANNELS_EXT;
use rusty_harness::harness::core_loop::plan::{Plan, Step, StepStatus};
use rusty_harness::harness::core_loop::task::{PLAN_EXT, TASK_EXT};
use rusty_harness::interaction::renderer::a2ui::{
    self, A2uiMessage, Action, ActionEvent, BASIC_CATALOG, ButtonVariant, Children, ChoiceOption,
    ChoiceVariant, Component, ComponentKind, CreateSurface, Dynamic, EventAction, TextVariant,
    UpdateComponents, UpdateDataModel,
};
use rusty_harness::interaction::renderer::tui::{
    App, Footer, ModelEntry, ModelTrait, Outgoing, Theme, TuiOptions, view,
};
use serde_json::{Value, json};

const TASK: &str = "task_01HZX4V3K9N2";
const CONTEXT: &str = "ctx_9a1f";

fn at(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 24, h, m, 0).unwrap()
}

fn options() -> TuiOptions {
    TuiOptions {
        agent: "demo-agent".into(),
        locale: "en".into(),
        models: vec![
            entry(
                "gpt-6.1",
                "openai/gpt-6.1",
                &[ModelTrait::Tools, ModelTrait::Vision, ModelTrait::Voice],
            ),
            entry(
                "gpt-6.1-mini",
                "openai/gpt-6.1-mini",
                &[ModelTrait::Tools, ModelTrait::Vision],
            ),
            entry(
                "fable",
                "anthropic/claude-fable-5-1",
                &[ModelTrait::Tools, ModelTrait::Vision],
            ),
            entry("qwen", "openai/qwen3-coder", &[ModelTrait::Tools]),
            entry("local", "openai/llama4", &[ModelTrait::Tools]),
        ],
        commands: vec![
            "model".into(),
            "effort".into(),
            "exit".into(),
            "plan".into(),
        ],
        footer: Footer {
            model: "gpt-6.1".into(),
            effort: Some("high".into()),
            durable: "remote".into(),
        },
        mono: false,
        voice: false,
        push_to_talk: ' ',
        hold: false,
    }
}

fn entry(alias: &str, label: &str, traits: &[ModelTrait]) -> ModelEntry {
    ModelEntry {
        alias: alias.into(),
        label: label.into(),
        traits: traits.to_vec(),
    }
}

fn task_meta(sub_state: Option<Value>) -> Value {
    let mut meta = json!({
        "name": "release notes",
        "goal": "draft the Q4 release notes",
        "description": "",
        "acceptance_criteria": [],
        "mode": "long-running",
        "sub_tasks": [{ "task": "classify-25-prs" }],
        "parent_tasks": [],
        "reference_task_ids": []
    });
    if let Some(sub) = sub_state {
        meta["sub_state"] = sub;
    }
    meta
}

fn plan(done: usize, running: usize) -> Value {
    let mut plan = Plan::new("p1", TASK);
    for (i, name) in ["collect", "group", "draft", "review"].iter().enumerate() {
        let status = if i < done {
            StepStatus::Done
        } else if i < done + running {
            StepStatus::Running
        } else {
            StepStatus::Pending
        };
        plan.add_step(Step {
            id: (*name).into(),
            name: (*name).into(),
            description: String::new(),
            status,
            output: None,
            linked_tasks: vec![],
        })
        .unwrap();
    }
    serde_json::to_value(&plan).unwrap()
}

fn metadata(sub_state: Option<Value>, plan_value: Value) -> HashMap<String, Value> {
    HashMap::from([
        (TASK_EXT.to_owned(), task_meta(sub_state)),
        (PLAN_EXT.to_owned(), plan_value),
    ])
}

fn status(
    state: a2a::TaskState,
    message: Option<a2a::Message>,
    sub_state: Option<Value>,
    plan_value: Value,
    when: DateTime<Utc>,
) -> a2a::StreamResponse {
    let meta = metadata(sub_state, plan_value);
    let message = message.map(|mut m| {
        let mut md = m.metadata.take().unwrap_or_default();
        md.extend(meta.clone());
        m.metadata = Some(md);
        m
    });
    a2a::StreamResponse::StatusUpdate(a2a::TaskStatusUpdateEvent {
        task_id: TASK.into(),
        context_id: CONTEXT.into(),
        status: a2a::TaskStatus {
            state,
            message,
            timestamp: Some(when),
        },
        metadata: Some(meta),
    })
}

fn agent_text(id: &str, text: &str) -> a2a::Message {
    let mut m = a2a::Message::new(a2a::Role::Agent, vec![a2a::Part::text(text)]);
    m.message_id = id.into();
    m.task_id = Some(TASK.into());
    m.context_id = Some(CONTEXT.into());
    m
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn type_text(app: &mut App, text: &str, when: DateTime<Utc>) -> Vec<Outgoing> {
    let mut out = Vec::new();
    for c in text.chars() {
        out.extend(app.key(key(KeyCode::Char(c)), when));
    }
    out
}

/// The task as the stream opens it: a working task with a plan and a sub-task.
fn working_task() -> a2a::StreamResponse {
    a2a::StreamResponse::Task(a2a::Task {
        id: TASK.into(),
        context_id: CONTEXT.into(),
        status: a2a::TaskStatus {
            state: a2a::TaskState::Working,
            message: None,
            timestamp: Some(at(12, 30)),
        },
        artifacts: None,
        history: None,
        metadata: Some(metadata(None, plan(1, 1))),
    })
}

/// The prototype's `chat` state: a turn each way, the reply still streaming.
fn chat_app() -> App {
    let mut app = App::new(options());
    let sent = type_text(
        &mut app,
        "Draft release notes for Q4. Keep the task open, I will iterate with you.",
        at(12, 30),
    );
    assert_eq!(sent, Vec::new());
    let out = app.key(key(KeyCode::Enter), at(12, 30));
    assert!(matches!(out.as_slice(), [Outgoing::Send(_)]), "{out:?}");
    app.apply(working_task(), at(12, 30));
    app.apply(
        status(
            a2a::TaskState::Working,
            Some(agent_text(
                "m2",
                "Three areas changed since v0.3.0:\n\n- API: per-tenant rate limits (#401), pagination on /events (#404)\n- Auth: refresh-token rotation (#407)\n- CLI: the new `--config` flag",
            )),
            None,
            plan(1, 1),
            at(12, 31),
        ),
        at(12, 31),
    );
    app
}

fn ids(ids: &[&str]) -> Children {
    Children::Ids(ids.iter().map(|s| (*s).to_owned()).collect())
}

/// The ask of the TUI prototype (`design/tui-renderer.html`, state `a2ui`).
#[allow(clippy::too_many_lines)]
fn audience_form() -> Vec<A2uiMessage> {
    let text = |id: &str, t: &str, variant: Option<TextVariant>| {
        Component::new(
            id,
            ComponentKind::Text {
                text: t.into(),
                variant,
            },
        )
    };
    let button = |id: &str, _label: &str, variant: Option<ButtonVariant>, action: Action| {
        Component::new(
            id,
            ComponentKind::Button {
                child: format!("{id}_label"),
                variant,
                action,
                checks: None,
            },
        )
    };
    let components = vec![
        Component::new(
            "root",
            ComponentKind::Column {
                children: ids(&["question", "detail", "audience", "other", "actions"]),
                justify: None,
                align: None,
            },
        ),
        text(
            "question",
            "Which audience are these notes for?",
            Some(TextVariant::H3),
        ),
        text(
            "detail",
            "The tone and the level of detail depend on it. Pick one, or type another.",
            Some(TextVariant::Body),
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
        button(
            "send",
            "Send answer",
            Some(ButtonVariant::Primary),
            Action::Event(EventAction {
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
        ),
        text("send_label", "Send answer", None),
        button(
            "skip",
            "Skip",
            Some(ButtonVariant::Borderless),
            Action::event("skip"),
        ),
        text("skip_label", "Skip", None),
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
        A2uiMessage::UpdateDataModel(UpdateDataModel {
            surface_id: "ask-ch-7f3a".into(),
            path: None,
            value: Some(json!({ "audience": ["customers"], "other": "" })),
        }),
    ]
}

/// The question as the harness's status message carries it (task 33's shape).
fn ask_message() -> a2a::Message {
    let form = audience_form();
    let mut alt = a2a::Part::text(a2ui::text_alternative(&form));
    alt.metadata = Some(HashMap::from([(
        a2ui::ALTERNATIVE_FOR_KEY.to_owned(),
        json!(a2ui::MEDIA_TYPE),
    )]));
    let mut m = a2a::Message::new(a2a::Role::Agent, vec![alt, a2ui::part(&form)]);
    m.message_id = "msg_ask".into();
    m.task_id = Some(TASK.into());
    m.context_id = Some(CONTEXT.into());
    m.extensions = Some(vec![a2ui::EXTENSION_URI.to_owned()]);
    m.metadata = Some(HashMap::from([
        (
            CHANNELS_EXT.to_owned(),
            json!({ "channel": "ch-7f3a", "ask": "ch-7f3a", "to": ["you"], "question_kind": "a2ui" }),
        ),
        (a2ui::EXTENSION_URI.to_owned(), json!({})),
    ]));
    m
}

/// The prototype's `a2ui` state: the task waits on the person's answer to a form.
fn a2ui_app() -> App {
    let mut app = App::new(options());
    app.apply(working_task(), at(12, 30));
    app.apply(
        status(
            a2a::TaskState::InputRequired,
            Some(ask_message()),
            Some(json!({ "kind": "awaiting-participant", "ask": "ch-7f3a" })),
            plan(2, 1),
            at(12, 33),
        ),
        at(12, 33),
    );
    app
}

/// The task after a delivered turn: `input-required`, awaiting the next message.
fn delivered_app() -> App {
    let mut app = App::new(options());
    app.apply(working_task(), at(12, 30));
    app.apply(
        status(
            a2a::TaskState::InputRequired,
            Some(agent_text(
                "m3",
                "Done: the draft is attached as release-notes-q4.md. Tell me what to change, or complete.",
            )),
            Some(json!({ "kind": "awaiting-next-turn" })),
            plan(3, 1),
            at(12, 35),
        ),
        at(12, 35),
    );
    app
}

/// A painted frame: its glyphs, whether any cell carries a colour, and how many cells
/// carry a background colour (the half-block image encoder paints pixels that way).
struct Rendered {
    text: String,
    coloured: bool,
    painted: usize,
}

fn paint(app: &mut App, theme: Theme) -> Rendered {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| view::draw_with(app, frame, theme))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    let mut coloured = false;
    let mut painted = 0;
    for y in 0..30u16 {
        let mut line = String::new();
        for x in 0..100u16 {
            let cell = buffer.cell((x, y)).unwrap();
            line.push_str(cell.symbol());
            if cell.fg != Color::Reset || cell.bg != Color::Reset {
                coloured = true;
            }
            if cell.bg != Color::Reset {
                painted += 1;
            }
        }
        text.push_str(line.trim_end());
        text.push('\n');
    }
    Rendered {
        text,
        coloured,
        painted,
    }
}

/// The buffer as text and whether any cell carries a colour.
fn render(app: &mut App, theme: Theme) -> (String, bool) {
    let r = paint(app, theme);
    (r.text, r.coloured)
}

/// T5 + T9: the coloured buffer is the snapshot; the `NO_COLOR` buffer shows the same
/// glyphs with no colour at all.
fn snapshot(name: &str, mut app: App) -> String {
    let (text, coloured) = render(&mut app, Theme::color());
    assert!(coloured, "{name}: the colour theme sets colours");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/snapshots/tui_{name}.txt"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let published = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        published, text,
        "tui_{name}.txt is stale: rerun with UPDATE_SNAPSHOTS=1\n{text}"
    );
    let (mono, coloured) = render(&mut app, Theme::mono());
    assert!(!coloured, "{name}: NO_COLOR leaves no colour behind");
    assert_eq!(
        mono, text,
        "{name}: the monochrome buffer shows the same glyphs"
    );
    text
}

#[test]
fn tui_snapshot_chat() {
    let text = snapshot("chat", chat_app());
    assert!(text.contains("● working"), "{text}");
    assert!(text.contains("you 12:30"), "{text}");
    assert!(text.contains("demo-agent 12:31 · streaming"), "{text}");
    assert!(
        text.contains("■ collect ▣ group □ draft □ review"),
        "{text}"
    );
    assert!(text.contains("sub-task classify-25-prs"), "{text}");
    assert!(text.contains("gpt-6.1 · high · remote"), "{text}");
}

#[test]
fn tui_snapshot_a2ui() {
    let text = snapshot("a2ui", a2ui_app());
    assert!(
        text.contains("◐ input-required awaiting participant"),
        "{text}"
    );
    assert!(text.contains("asked you on channel ch-7f3a"), "{text}");
    assert!(text.contains("a2ui · basic catalog"), "{text}");
    assert!(
        text.contains("(●) Customers   ( ) Internal engineering   ( ) Both, two sections"),
        "{text}"
    );
    assert!(text.contains("Something else"), "{text}");
    assert!(text.contains("Send answer"), "{text}");
    assert!(text.contains("[ Skip ]"), "{text}");
    assert!(text.contains("tab  next field"), "{text}");
}

#[test]
fn tui_snapshot_model() {
    let mut app = delivered_app();
    assert_eq!(type_text(&mut app, "/model", at(12, 35)), Vec::new());
    assert!(app.matching_commands().contains(&"/model".to_owned()));
    let out = app.key(key(KeyCode::Enter), at(12, 35));
    assert!(
        out.is_empty(),
        "the picker opens instead of sending: {out:?}"
    );
    let text = snapshot("model", app);
    assert!(text.contains("pick the model for the next call"), "{text}");
    assert!(text.contains("▸ openai/gpt-6.1 (gpt-6.1)"), "{text}");
    assert!(text.contains("current · tools · vision · voice"), "{text}");
}

#[test]
fn tui_snapshot_done() {
    let mut app = delivered_app();
    let out = app.key(ctrl('c'), at(12, 35));
    assert!(out.is_empty(), "^c asks first: {out:?}");
    let text = snapshot("done", app);
    assert!(text.contains("complete task?"), "{text}");
    assert!(
        text.contains("1 running sub-task will be canceled"),
        "{text}"
    );
    assert!(text.contains("y  complete"), "{text}");
}

#[test]
fn tui_snapshot_first() {
    let mut opts = options();
    opts.footer.durable = "none".into();
    opts.footer.effort = None;
    let text = snapshot("first", App::new(opts));
    assert!(text.contains("○ no task yet"), "{text}");
    assert!(
        text.contains("Type a message to start a task. Prefix with / for commands."),
        "{text}"
    );
}

fn voiced(hold: bool) -> App {
    let mut opts = options();
    opts.voice = true;
    opts.hold = hold;
    let mut app = App::new(opts);
    app.apply(working_task(), at(12, 30));
    app.apply(
        status(
            a2a::TaskState::InputRequired,
            Some(agent_text(
                "m3",
                "Done: the draft is attached as release-notes-q4.md. Tell me what to change, or complete.",
            )),
            Some(json!({ "kind": "awaiting-next-turn" })),
            plan(3, 1),
            at(12, 35),
        ),
        at(12, 35),
    );
    app
}

fn space(kind: KeyEventKind) -> KeyEvent {
    KeyEvent::new_with_kind(KeyCode::Char(' '), KeyModifiers::NONE, kind)
}

#[test]
fn tui_snapshot_voice() {
    let mut app = voiced(true);
    assert_eq!(
        app.key(space(KeyEventKind::Press), at(12, 36)),
        vec![Outgoing::StartListening]
    );
    app.tick(at(12, 36) + chrono::Duration::seconds(4));
    let text = snapshot("voice", app);
    assert!(
        text.contains("● listening 0:04 · release space to send"),
        "{text}"
    );
    assert!(
        text.contains("space  held (kitty keyboard protocol)"),
        "{text}"
    );
}

/// Scenario: Push-to-talk holds with key releases and toggles without them
#[test]
fn push_to_talk_holds_with_releases_and_toggles_without() {
    // Held: press starts, repeats are ignored, release sends.
    let mut app = voiced(true);
    assert_eq!(
        app.key(space(KeyEventKind::Press), at(12, 36)),
        vec![Outgoing::StartListening]
    );
    assert_eq!(app.key(space(KeyEventKind::Repeat), at(12, 36)), vec![]);
    assert_eq!(
        app.key(space(KeyEventKind::Release), at(12, 37)),
        vec![Outgoing::StopListening { send: true }]
    );
    assert!(app.listening.is_none());

    // Toggled: a press starts, the release is ignored, the next press sends.
    let mut app = voiced(false);
    assert_eq!(
        app.key(space(KeyEventKind::Press), at(12, 36)),
        vec![Outgoing::StartListening]
    );
    assert_eq!(app.key(space(KeyEventKind::Release), at(12, 36)), vec![]);
    assert_eq!(
        app.key(space(KeyEventKind::Press), at(12, 37)),
        vec![Outgoing::StopListening { send: true }]
    );

    // Esc drops a capture.
    let mut app = voiced(false);
    app.key(space(KeyEventKind::Press), at(12, 36));
    assert_eq!(
        app.key(key(KeyCode::Esc), at(12, 36)),
        vec![Outgoing::StopListening { send: false }]
    );

    // With text in the composer, space is a space.
    let mut app = voiced(true);
    type_text(&mut app, "add", at(12, 36));
    assert_eq!(app.key(space(KeyEventKind::Press), at(12, 36)), vec![]);
    assert_eq!(app.composer, "add ");

    // Without a microphone, space types.
    let mut app = App::new(options());
    assert_eq!(app.key(space(KeyEventKind::Press), at(12, 36)), vec![]);
    assert_eq!(app.composer, " ");
}

#[test]
fn a_capture_is_sent_as_a_voice_part_and_a_spoken_reply_is_queued_for_playback() {
    use rusty_harness::harness::models::speech::{VOICE_EXT, audio_of};
    let mut app = voiced(true);
    let message = app.spoken_message(vec![1, 2, 3, 4], at(12, 37));
    assert_eq!(message.task_id.as_deref(), Some(TASK));
    let (meta, bytes) = audio_of(&message.parts[0]).expect("a voice/v1 part");
    assert_eq!(bytes.as_ref(), [1, 2, 3, 4]);
    assert!(meta.is_final);
    assert_eq!(message.parts[0].media_type.as_deref(), Some("audio/pcm"));
    assert!(
        message.parts[0]
            .metadata
            .as_ref()
            .unwrap()
            .contains_key(VOICE_EXT)
    );

    let mut part = a2a::Part::data(Value::Null).with_media_type("audio/pcm");
    part.content = a2a::PartContent::Raw(vec![9, 8, 7]);
    app.apply(
        a2a::StreamResponse::ArtifactUpdate(a2a::TaskArtifactUpdateEvent {
            task_id: TASK.into(),
            context_id: CONTEXT.into(),
            artifact: a2a::Artifact {
                artifact_id: "speech-3-1".into(),
                name: Some("reply.pcm".into()),
                description: Some("the reply, spoken".into()),
                parts: vec![part],
                metadata: None,
                extensions: None,
            },
            append: None,
            last_chunk: Some(true),
            metadata: None,
        }),
        at(12, 38),
    );
    let queued = app.take_playback();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].as_ref(), [9, 8, 7]);
    assert!(app.take_playback().is_empty(), "played once");
}

/// Scenario: A command token is highlighted in the composer and in a sent message
#[test]
fn a_registered_command_is_highlighted_and_an_unknown_one_is_not() {
    let app = App::new(options());
    assert_eq!(app.command_tokens("/model gpt"), vec![(0, 6)]);
    assert_eq!(app.command_tokens("/nope"), Vec::<(usize, usize)>::new());
    assert_eq!(
        app.command_tokens("hello /model"),
        Vec::<(usize, usize)>::new()
    );
}

/// Scenario: Answering an A2UI form sends the action as an A2UI client message on the
/// channel
#[test]
fn an_a2ui_form_answer_becomes_a_client_action_on_the_channel() {
    let mut app = a2ui_app();
    // Focus order: the choice picker, the text field, then the two buttons.
    app.key(key(KeyCode::Tab), at(12, 34));
    assert_eq!(type_text(&mut app, "partners", at(12, 34)), Vec::new());
    app.key(key(KeyCode::Tab), at(12, 34));
    let out = app.key(key(KeyCode::Enter), at(12, 34));
    let [Outgoing::Send(message)] = out.as_slice() else {
        panic!("one send: {out:?}");
    };
    assert_eq!(message.task_id.as_deref(), Some(TASK));
    let meta = message.metadata.as_ref().unwrap();
    assert_eq!(meta[CHANNELS_EXT]["reply_to"], "ch-7f3a");
    assert_eq!(
        meta[a2ui::CLIENT_CAPABILITIES_KEY]["v0.9.1"]["supportedCatalogIds"][0],
        BASIC_CATALOG,
        "the TUI declares the catalog it renders"
    );
    let part = &message.parts[0];
    assert!(a2ui::is_a2ui(part));
    let a2a::PartContent::Data(data) = &part.content else {
        panic!("a data part")
    };
    assert_eq!(data[0]["version"], "v0.9.1");
    assert_eq!(data[0]["action"]["name"], "answer");
    assert_eq!(data[0]["action"]["surfaceId"], "ask-ch-7f3a");
    assert_eq!(data[0]["action"]["sourceComponentId"], "send");
    assert_eq!(
        data[0]["action"]["context"]["audience"],
        json!(["customers"])
    );
    assert_eq!(data[0]["action"]["context"]["other"], "partners");
}

/// Scenario: A completed task is followed up as a new task that references it
#[test]
fn a_message_after_a_terminal_task_starts_a_new_task_that_references_it() {
    let mut app = delivered_app();
    app.apply(
        status(
            a2a::TaskState::Completed,
            None,
            None,
            plan(4, 0),
            at(12, 40),
        ),
        at(12, 40),
    );
    type_text(&mut app, "thanks, one more thing", at(12, 41));
    let out = app.key(key(KeyCode::Enter), at(12, 41));
    let [Outgoing::Send(message)] = out.as_slice() else {
        panic!("{out:?}")
    };
    assert_eq!(message.task_id, None);
    assert_eq!(message.context_id.as_deref(), Some(CONTEXT));
    assert_eq!(message.reference_task_ids, Some(vec![TASK.to_owned()]));
    assert_eq!(
        message.metadata.as_ref().unwrap()[TASK_EXT]["mode"],
        "long-running"
    );
}

/// Scenario: Complete sends the task extension's transition and cancel sends `CancelTask`
#[test]
fn complete_and_cancel_are_explicit_controls() {
    let mut app = delivered_app();
    app.key(ctrl('c'), at(12, 36));
    let out = app.key(key(KeyCode::Char('y')), at(12, 36));
    let [Outgoing::Send(message)] = out.as_slice() else {
        panic!("{out:?}")
    };
    assert_eq!(
        message.metadata.as_ref().unwrap()[TASK_EXT]["transition"],
        "complete"
    );
    assert_eq!(message.task_id.as_deref(), Some(TASK));

    let mut app = delivered_app();
    assert_eq!(
        app.key(ctrl('x'), at(12, 36)),
        vec![Outgoing::Cancel(TASK.into())]
    );
}

fn png() -> Vec<u8> {
    // Larger than a terminal cell (the picker assumes 10×20 px), so there are pixels
    // to draw once the image is fitted to the cell grid.
    let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        200,
        100,
        image::Rgba([220, 40, 40, 255]),
    ));
    let mut bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .unwrap();
    bytes
}

fn image_artifact() -> a2a::StreamResponse {
    let mut part = a2a::Part::data(json!(null)).with_media_type("image/png");
    part.content = a2a::PartContent::Raw(png());
    a2a::StreamResponse::ArtifactUpdate(a2a::TaskArtifactUpdateEvent {
        task_id: TASK.into(),
        context_id: CONTEXT.into(),
        artifact: a2a::Artifact {
            artifact_id: "a1".into(),
            name: Some("chart.png".into()),
            description: Some("a red square".into()),
            parts: vec![part],
            metadata: None,
            extensions: None,
        },
        append: None,
        last_chunk: Some(true),
        metadata: None,
    })
}

/// Scenario: An image artifact renders in a graphics-capable terminal and degrades to alt text
///     Given an image artifact on the task
///     When the terminal offers a graphics protocol
///     Then the image is drawn
///     When the terminal offers none
///     Then the artifact's name and alt text are shown instead
#[test]
fn an_image_artifact_renders_in_a_graphics_capable_terminal_and_degrades_to_alt_text() {
    let mut drawn = delivered_app();
    drawn.set_picker(Some(ratatui_image::picker::Picker::halfblocks()));
    drawn.apply(image_artifact(), at(12, 36));
    let frame = paint(&mut drawn, Theme::color());
    assert!(
        frame.text.contains("image chart.png: a red square"),
        "{}",
        frame.text
    );
    // 200×100 px at the picker's 10×20 px cell is a 20×5 block of painted cells.
    assert_eq!(
        frame.painted, 100,
        "pixels painted as half blocks:\n{}",
        frame.text
    );

    let mut plain = delivered_app();
    plain.apply(image_artifact(), at(12, 36));
    assert!(!plain.draws_images());
    let frame = paint(&mut plain, Theme::color());
    assert!(
        frame.text.contains("artifact chart.png: a red square"),
        "{}",
        frame.text
    );
    assert_eq!(frame.painted, 0, "{}", frame.text);
}

#[test]
fn a_remote_card_supplies_the_commands_and_the_model_aliases() {
    use rusty_harness::interaction::renderer::tui::card_commands;
    // The snapshot of the card the server builds from the default configuration.
    let snapshot = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/agent_card.json"),
    )
    .unwrap();
    let card: a2a::AgentCard = serde_json::from_str(&snapshot).unwrap();
    let (commands, models) = card_commands(&card);
    assert!(commands.contains(&"model".to_owned()), "{commands:?}");
    assert!(commands.contains(&"effort".to_owned()), "{commands:?}");
    assert_eq!(models, ["default"]);
    let mut app = App::new(TuiOptions {
        models: Vec::new(),
        commands: Vec::new(),
        ..options()
    });
    app.adopt_card(&card);
    assert_eq!(app.options().agent, card.name);
    assert_eq!(app.options().commands, commands);
    assert_eq!(
        app.options()
            .models
            .iter()
            .map(|m| m.alias.as_str())
            .collect::<Vec<_>>(),
        ["default"]
    );
    let mut bare = card;
    bare.capabilities.extensions = None;
    assert_eq!(card_commands(&bare), (Vec::new(), Vec::new()));
}

/// The TUI as an A2A client of a running harness, headless: the same transport the
/// binary uses, the stream reduced by `App`.
#[cfg(feature = "server")]
mod on_one_harness {
    use std::sync::Arc;

    use futures::StreamExt as _;
    use rusty_harness::app_service::a2a_server::serve;
    use rusty_harness::config::Config;
    use rusty_harness::harness::a2a_client::AgentTransport as _;
    use rusty_harness::harness::a2a_client::official::{OfficialTransport, fetch_card};
    use rusty_harness::harness::models::llm::testing::RecordedLlm;
    use rusty_harness::harness::models::llm::{ModelEvent, StopReason};
    use rusty_harness::harness::tools::ToolCall;
    use rusty_harness::host::Host;
    use rusty_harness::interaction::renderer::tui::{App, Outgoing};
    use rusty_harness::{Harness, HarnessBuilder};
    use serde_json::json;

    use super::{at, audience_form, key, options, type_text};
    use crossterm::event::KeyCode;

    fn text(t: &str) -> Vec<ModelEvent> {
        vec![
            ModelEvent::TextDelta { text: t.into() },
            ModelEvent::Stop {
                reason: StopReason::EndTurn,
            },
        ]
    }

    async fn harness(turns: Vec<Vec<ModelEvent>>) -> Harness {
        let mut config = Config::default();
        config.server.bind = "127.0.0.1:0".into();
        HarnessBuilder::new(config)
            .host(Host::testing())
            .llm(Arc::new(RecordedLlm::new("default", turns)))
            .build()
            .await
            .expect("builds")
    }

    /// Sends `outgoing` through the official client and feeds every event to `app`.
    async fn drive(app: &mut App, transport: &OfficialTransport, outgoing: Vec<Outgoing>) {
        for out in outgoing {
            let Outgoing::Send(message) = out else {
                panic!("{out:?}")
            };
            let request = a2a::SendMessageRequest {
                message: *message,
                configuration: None,
                metadata: None,
                tenant: None,
            };
            let mut stream = transport
                .send_streaming_message(request, Vec::new())
                .await
                .expect("stream");
            while let Some(item) = stream.next().await {
                app.apply(item.expect("event"), at(12, 30));
            }
        }
    }

    /// Scenario: An A2UI ask renders in the web renderer and the TUI (the TUI leg)
    ///     Given a harness whose model asks an A2UI question on a channel, then answers
    ///     When the TUI sends a message through the official A2A client
    ///     Then the ask renders as a form and the form's answer resumes the task
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_a2ui_ask_renders_in_the_tui_on_one_harness() {
        let ask = vec![
            ModelEvent::ToolCall {
                call: ToolCall {
                    id: "c1".into(),
                    name: "participant.ask".into(),
                    arguments: json!({
                        "to": ["operator"],
                        "kind": "a2ui",
                        "question": audience_form(),
                    }),
                },
            },
            ModelEvent::Stop {
                reason: StopReason::ToolUse,
            },
        ];
        let h = harness(vec![ask, text("Drafting for customers.")]).await;
        let server = serve(&h, None).await.expect("binds");
        let base: url::Url = format!("http://{}/", server.addr).parse().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(server.run(async move {
            let _ = stopped.await;
        }));

        // The client side is a real host: the testing host's HTTP client reaches nothing.
        let client_host = Host::native();
        let card = fetch_card(&client_host, &base).await.expect("card");
        let mut app = App::new(options());
        app.adopt_card(&card);
        let transport = OfficialTransport::from_card(&card, None, Vec::new()).expect("transport");

        type_text(&mut app, "Draft release notes for Q4.", at(12, 30));
        let out = app.key(key(KeyCode::Enter), at(12, 30));
        drive(&mut app, &transport, out).await;
        let (text, _) = super::render(&mut app, super::Theme::color());
        assert!(text.contains("a2ui · basic catalog"), "{text}");
        assert!(
            text.contains("Which audience are these notes for?"),
            "{text}"
        );
        assert!(
            text.contains("◐ input-required awaiting participant"),
            "{text}"
        );

        // Answer: the choice picker is focused first; tab twice to "Send answer".
        app.key(key(KeyCode::Tab), at(12, 31));
        app.key(key(KeyCode::Tab), at(12, 31));
        let out = app.key(key(KeyCode::Enter), at(12, 31));
        assert_eq!(out.len(), 1, "one reply on the channel: {out:?}");
        drive(&mut app, &transport, out).await;
        let (text, _) = super::render(&mut app, super::Theme::color());
        assert!(text.contains("Drafting for customers."), "{text}");
        assert!(
            text.contains("◐ input-required awaiting next turn"),
            "{text}"
        );

        let _ = stop.send(());
        let _ = server_task.await;
    }
}

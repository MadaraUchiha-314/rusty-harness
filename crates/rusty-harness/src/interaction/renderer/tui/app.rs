//! The TUI's state and reducer: A2A events in, requests out (R27.3–4, R27.6).

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde_json::Value;

use super::form::Form;
use crate::assets::{Catalog, FmtArgs, StringKey, keys};
use crate::harness::channels::{CHANNELS_EXT, ChannelsMeta, QuestionKind};
use crate::harness::commands::{COMMANDS_EXT, CommandsMeta};
use crate::harness::core_loop::plan::{Plan, StepStatus};
use crate::harness::core_loop::task::{PLAN_EXT, SubState, TASK_EXT, TaskId, TaskMeta};
use crate::interaction::renderer::a2ui;
use crate::interaction::renderer::{RenderItem, a2ui::BASIC_CATALOG};
use crate::interaction::surface::SurfaceCapabilities;

/// What a model can do, for the picker's right column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelTrait {
    /// Tool calling.
    Tools,
    /// Image input.
    Vision,
    /// Audio input.
    Voice,
}

impl ModelTrait {
    fn key(self) -> StringKey {
        match self {
            Self::Tools => keys::tui::model::ability::TOOLS,
            Self::Vision => keys::tui::model::ability::VISION,
            Self::Voice => keys::tui::model::ability::VOICE,
        }
    }
}

/// One row of the `/model` picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelEntry {
    /// The configured alias (`/model <alias>`).
    pub alias: String,
    /// `provider/model`.
    pub label: String,
    /// Its traits.
    pub traits: Vec<ModelTrait>,
}

/// The right end of the hint bar: what runs the next call and where tasks live.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Footer {
    /// The model alias.
    pub model: String,
    /// The effort, when set.
    pub effort: Option<String>,
    /// The durable mode (`none`, `embedded`, `remote`).
    pub durable: String,
}

/// What the TUI is told at start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuiOptions {
    /// The agent's display name (the card's).
    pub agent: String,
    /// The locale of every string shown.
    pub locale: String,
    /// The `/model` picker rows; empty for a remote harness, whose list is not known.
    pub models: Vec<ModelEntry>,
    /// Registered command names, for highlighting (R27.6).
    pub commands: Vec<String>,
    /// The hint bar's right end.
    pub footer: Footer,
    /// Monochrome (`NO_COLOR`).
    pub mono: bool,
}

impl Default for TuiOptions {
    fn default() -> Self {
        Self {
            agent: "rusty-harness".into(),
            locale: "en".into(),
            models: Vec::new(),
            commands: vec!["model".into(), "effort".into(), "exit".into()],
            footer: Footer::default(),
            mono: false,
        }
    }
}

/// What the TUI wants sent.
#[derive(Clone, Debug, PartialEq)]
pub enum Outgoing {
    /// `SendStreamingMessage`.
    Send(Box<a2a::Message>),
    /// `CancelTask`.
    Cancel(TaskId),
    /// Leave.
    Quit,
}

/// What arrives from the harness.
#[derive(Debug)]
pub enum Incoming {
    /// A stream event.
    Response(Box<a2a::StreamResponse>),
    /// A failed request or stream.
    Error(String),
}

/// Who wrote a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    /// The person at the terminal.
    You,
    /// The agent.
    Agent,
}

/// A decoded image artifact, as the terminal protocol keeps it.
pub struct ImageState {
    /// The encoder state; drawn by `StatefulImage`.
    pub protocol: ratatui_image::protocol::StatefulProtocol,
}

impl core::fmt::Debug for ImageState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ImageState").finish_non_exhaustive()
    }
}

/// One thing in the conversation.
#[derive(Debug)]
pub enum Entry {
    /// A message turn.
    Turn {
        /// Who.
        who: Who,
        /// When it arrived here.
        at: DateTime<Utc>,
        /// Markdown.
        text: String,
        /// Command tokens to highlight (byte ranges).
        tokens: Vec<(usize, usize)>,
    },
    /// An A2UI question on a channel.
    Ask {
        /// The channel.
        channel: String,
        /// When.
        at: DateTime<Utc>,
        /// The form.
        form: Form,
    },
    /// An artifact: an image when the terminal can draw it, its text otherwise.
    Artifact {
        /// Its name.
        name: String,
        /// Its alt text (the artifact's description or the part's).
        alt: String,
        /// The decoded image.
        image: Option<ImageState>,
    },
    /// A note from the harness: a command outcome, an error.
    Notice {
        /// When.
        at: DateTime<Utc>,
        /// The text.
        text: String,
    },
}

/// What the keys do right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Typing a message.
    Chat,
    /// Inside the newest A2UI form.
    Form,
    /// The `/model` picker.
    ModelPicker {
        /// The highlighted row.
        selected: usize,
    },
    /// The complete confirmation.
    ConfirmComplete,
}

/// The task shown in the header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskHeader {
    /// The task.
    pub id: TaskId,
    /// Its context.
    pub context_id: String,
    /// Its state.
    pub state: a2a::TaskState,
    /// Its sub-state.
    pub sub_state: Option<SubState>,
    /// Its sub-tasks, by id.
    pub sub_tasks: Vec<String>,
}

/// The TUI's state (design.md § TUI renderer and CLI).
pub struct App {
    opts: TuiOptions,
    catalog: Catalog,
    surface: SurfaceCapabilities,
    /// The current task, once the first turn created one.
    pub task: Option<TaskHeader>,
    /// The last task that ended, which the next message references (R21.7).
    pub last_task: Option<TaskId>,
    /// The conversation, oldest first.
    pub entries: Vec<Entry>,
    /// The plan strip: step names and statuses.
    pub plan: Vec<(String, StepStatus)>,
    /// What is typed.
    pub composer: String,
    /// What the keys do.
    pub mode: Mode,
    /// A turn is in flight.
    pub busy: bool,
    /// Whether the person asked to leave.
    pub quit: bool,
    seen_messages: BTreeSet<String>,
    picker: Option<ratatui_image::picker::Picker>,
    scroll: u16,
}

impl core::fmt::Debug for App {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("App")
            .field("task", &self.task)
            .field("mode", &self.mode)
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

static SURFACE: OnceLock<SurfaceCapabilities> = OnceLock::new();

impl App {
    /// A fresh TUI with no task.
    #[must_use]
    pub fn new(opts: TuiOptions) -> Self {
        Self {
            opts,
            catalog: Catalog::builtin(),
            surface: Self::capabilities(),
            task: None,
            last_task: None,
            entries: Vec::new(),
            plan: Vec::new(),
            composer: String::new(),
            mode: Mode::Chat,
            busy: false,
            quit: false,
            seen_messages: BTreeSet::new(),
            picker: None,
            scroll: 0,
        }
    }

    /// What the TUI renders: text, images, and the A2UI basic catalog.
    #[must_use]
    pub fn capabilities() -> SurfaceCapabilities {
        SurfaceCapabilities::text_only()
            .with_output("image/*")
            .with_a2ui(BASIC_CATALOG)
    }

    pub(super) fn surface_static() -> &'static SurfaceCapabilities {
        SURFACE.get_or_init(Self::capabilities)
    }

    /// The options.
    #[must_use]
    pub fn options(&self) -> &TuiOptions {
        &self.opts
    }

    /// The string `key` in the TUI's locale.
    #[must_use]
    pub fn text(&self, key: StringKey) -> String {
        self.catalog.get(key, &self.opts.locale, &FmtArgs::new())
    }

    /// The string `key` with `args`.
    #[must_use]
    pub fn text_with(&self, key: StringKey, args: &FmtArgs) -> String {
        self.catalog.get(key, &self.opts.locale, args)
    }

    /// Draw images through `picker` (a graphics-capable terminal); without one, image
    /// artifacts show their alt text (R28.2).
    pub fn set_picker(&mut self, picker: Option<ratatui_image::picker::Picker>) {
        self.picker = picker;
    }

    /// Whether images are drawn.
    #[must_use]
    pub fn draws_images(&self) -> bool {
        self.picker.is_some()
    }

    /// Whether `item` is within the capabilities (see `Renderer::supports`).
    #[must_use]
    pub fn supports(&self, item: &RenderItem) -> bool {
        match item {
            RenderItem::McpApp(_) => self.surface.mcp_apps,
            RenderItem::A2ui(messages) => self.surface.renders_a2ui(a2ui::catalogs(messages)),
            _ => true,
        }
    }

    /// Renders an item (the `Renderer` path): stream objects as [`Self::apply`], an A2UI
    /// payload as a form without a channel.
    pub fn render_item(&mut self, item: RenderItem) {
        let now = Utc::now();
        match item {
            RenderItem::Message(m) => self.apply(a2a::StreamResponse::Message(m), now),
            RenderItem::Task(t) => self.apply(a2a::StreamResponse::Task(t), now),
            RenderItem::StatusUpdate(u) => self.apply(a2a::StreamResponse::StatusUpdate(u), now),
            RenderItem::ArtifactUpdate(u) => {
                self.apply(a2a::StreamResponse::ArtifactUpdate(u), now);
            }
            RenderItem::A2ui(messages) => {
                self.push_form(String::new(), Form::new(&messages), now);
            }
            RenderItem::McpApp(_) => {}
        }
    }

    /// Applies an incoming item at `now`.
    pub fn receive(&mut self, incoming: Incoming, now: DateTime<Utc>) {
        match incoming {
            Incoming::Response(r) => self.apply(*r, now),
            Incoming::Error(reason) => {
                self.busy = false;
                let text = self.text_with(
                    keys::tui::error::REQUEST,
                    &FmtArgs::new().text("reason", reason),
                );
                self.entries.push(Entry::Notice { at: now, text });
            }
        }
    }

    /// Applies one stream event at `now`.
    pub fn apply(&mut self, response: a2a::StreamResponse, now: DateTime<Utc>) {
        match response {
            a2a::StreamResponse::Task(task) => {
                self.header(
                    &task.id,
                    &task.context_id,
                    task.status.state,
                    task.metadata.as_ref(),
                );
                self.plan_from(task.metadata.as_ref());
                if let Some(history) = task.history {
                    for m in &history {
                        self.message(m, now);
                    }
                }
                if let Some(m) = &task.status.message {
                    self.message(m, now);
                }
                if let Some(artifacts) = &task.artifacts {
                    for a in artifacts {
                        self.artifact(a);
                    }
                }
                self.after_state();
            }
            a2a::StreamResponse::StatusUpdate(update) => {
                let meta = update
                    .status
                    .message
                    .as_ref()
                    .and_then(|m| m.metadata.clone())
                    .or(update.metadata.clone());
                self.header(
                    &update.task_id,
                    &update.context_id,
                    update.status.state,
                    meta.as_ref(),
                );
                self.plan_from(update.metadata.as_ref());
                if let Some(m) = &update.status.message {
                    let at = update.status.timestamp.unwrap_or(now);
                    self.message(m, at);
                }
                self.after_state();
            }
            a2a::StreamResponse::ArtifactUpdate(update) => self.artifact(&update.artifact),
            a2a::StreamResponse::Message(m) => self.message(&m, now),
        }
    }

    fn header(
        &mut self,
        id: &str,
        context_id: &str,
        state: a2a::TaskState,
        metadata: Option<&HashMap<String, Value>>,
    ) {
        let meta: Option<TaskMeta> = metadata
            .and_then(|m| m.get(TASK_EXT))
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let sub_state = metadata
            .and_then(|m| m.get(TASK_EXT))
            .and_then(|v| v.get("sub_state"))
            .and_then(|v| serde_json::from_value::<SubState>(v.clone()).ok());
        if self.task.as_ref().is_none_or(|t| t.id != id) {
            self.task = Some(TaskHeader {
                id: id.to_owned(),
                context_id: context_id.to_owned(),
                state: state.clone(),
                sub_state: None,
                sub_tasks: Vec::new(),
            });
        }
        let header = self.task.as_mut().expect("just set");
        header.state = state;
        header.sub_state = sub_state;
        if let Some(meta) = meta {
            header.sub_tasks = meta.sub_tasks.into_iter().map(|t| t.task).collect();
        }
    }

    fn plan_from(&mut self, metadata: Option<&HashMap<String, Value>>) {
        if let Some(plan) = metadata
            .and_then(|m| m.get(PLAN_EXT))
            .and_then(|v| serde_json::from_value::<Plan>(v.clone()).ok())
        {
            self.plan = plan
                .steps()
                .iter()
                .map(|s| (s.name.clone(), s.status))
                .collect();
        }
    }

    fn after_state(&mut self) {
        let Some(header) = &self.task else { return };
        if header.state.is_terminal() {
            self.busy = false;
            self.last_task = Some(header.id.clone());
            if self.mode == Mode::Form {
                self.mode = Mode::Chat;
            }
        } else if header.state == a2a::TaskState::InputRequired {
            self.busy = false;
        }
    }

    fn message(&mut self, message: &a2a::Message, at: DateTime<Utc>) {
        if !message.message_id.is_empty() && !self.seen_messages.insert(message.message_id.clone())
        {
            return;
        }
        let who = if message.role == a2a::Role::User {
            Who::You
        } else {
            Who::Agent
        };
        let channels = ChannelsMeta::of(message);
        let a2ui_messages = a2ui::messages_in(message);
        if who == Who::Agent
            && !a2ui_messages.is_empty()
            && channels
                .as_ref()
                .is_none_or(|c| c.question_kind == QuestionKind::A2ui)
        {
            let channel = channels.map(|c| c.channel).unwrap_or_default();
            self.push_form(channel, Form::new(&a2ui_messages), at);
            return;
        }
        let text: Vec<String> = message
            .parts
            .iter()
            .filter_map(|p| match &p.content {
                a2a::PartContent::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        let text = text.join("\n");
        let tokens = message
            .metadata
            .as_ref()
            .and_then(|m| m.get(COMMANDS_EXT))
            .and_then(|v| serde_json::from_value::<CommandsMeta>(v.clone()).ok())
            .map_or_else(
                || self.command_tokens(&text),
                |c| {
                    c.tokens
                        .iter()
                        .map(|t| (t.start, t.end))
                        .collect::<Vec<_>>()
                },
            );
        if let Some(result) = message
            .metadata
            .as_ref()
            .and_then(|m| m.get(COMMANDS_EXT))
            .and_then(|v| serde_json::from_value::<CommandsMeta>(v.clone()).ok())
            .and_then(|c| c.result)
            && result.outcome == "exit"
        {
            self.quit = true;
        }
        if !text.is_empty() {
            self.entries.push(Entry::Turn {
                who,
                at,
                text,
                tokens,
            });
        }
    }

    fn push_form(&mut self, channel: String, form: Form, at: DateTime<Utc>) {
        self.entries.push(Entry::Ask { channel, at, form });
        self.mode = Mode::Form;
    }

    fn artifact(&mut self, artifact: &a2a::Artifact) {
        let name = artifact
            .name
            .clone()
            .unwrap_or_else(|| artifact.artifact_id.clone());
        let picker = self.picker.as_ref();
        for part in &artifact.parts {
            let media = part.media_type.clone().unwrap_or_default();
            match &part.content {
                a2a::PartContent::Text(t) => {
                    self.entries.push(Entry::Turn {
                        who: Who::Agent,
                        at: Utc::now(),
                        text: t.clone(),
                        tokens: Vec::new(),
                    });
                }
                a2a::PartContent::Raw(bytes) if media.starts_with("image/") => {
                    let alt = artifact
                        .description
                        .clone()
                        .or_else(|| {
                            part.metadata
                                .as_ref()
                                .and_then(|m| m.get("alt"))
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .unwrap_or_default();
                    let image = picker.and_then(|picker| {
                        image::load_from_memory(bytes).ok().map(|img| ImageState {
                            protocol: picker.new_resize_protocol(img),
                        })
                    });
                    self.entries.push(Entry::Artifact {
                        name: name.clone(),
                        alt,
                        image,
                    });
                }
                _ => {
                    let alt = part
                        .filename
                        .clone()
                        .or_else(|| match &part.content {
                            a2a::PartContent::Url(u) => Some(u.clone()),
                            _ => None,
                        })
                        .unwrap_or(media);
                    self.entries.push(Entry::Artifact {
                        name: name.clone(),
                        alt,
                        image: None,
                    });
                }
            }
        }
    }

    /// Byte ranges of registered commands at the start of a line (R27.6).
    #[must_use]
    pub fn command_tokens(&self, text: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix('/') {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                    .collect();
                if self.opts.commands.contains(&name) {
                    let start = offset + (line.len() - trimmed.len());
                    out.push((start, start + 1 + name.len()));
                }
            }
            offset += line.len();
        }
        out
    }

    /// The registered commands the composer's text could become.
    #[must_use]
    pub fn matching_commands(&self) -> Vec<String> {
        let Some(rest) = self.composer.strip_prefix('/') else {
            return Vec::new();
        };
        let typed: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
        self.opts
            .commands
            .iter()
            .filter(|c| c.starts_with(&typed))
            .map(|c| format!("/{c}"))
            .collect()
    }

    /// The newest form, when the conversation has one.
    pub fn form_mut(&mut self) -> Option<(&String, &mut Form)> {
        self.entries.iter_mut().rev().find_map(|e| match e {
            Entry::Ask { channel, form, .. } => Some((&*channel, form)),
            _ => None,
        })
    }

    /// The scroll offset of the conversation.
    #[must_use]
    pub fn scroll(&self) -> u16 {
        self.scroll
    }

    /// Handles a key at `now`.
    pub fn key(&mut self, key: KeyEvent, now: DateTime<Utc>) -> Vec<Outgoing> {
        if key.kind == KeyEventKind::Release {
            return Vec::new();
        }
        match self.mode.clone() {
            Mode::Chat => self.key_chat(key, now),
            Mode::Form => self.key_form(key, now),
            Mode::ModelPicker { selected } => self.key_picker(key, selected, now),
            Mode::ConfirmComplete => self.key_confirm(key, now),
        }
    }

    fn key_chat(&mut self, key: KeyEvent, now: DateTime<Utc>) -> Vec<Outgoing> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match (key.code, ctrl) {
            (KeyCode::Char('c'), true) => {
                if self.task.as_ref().is_some_and(|t| !t.state.is_terminal()) {
                    self.mode = Mode::ConfirmComplete;
                    Vec::new()
                } else {
                    self.quit = true;
                    vec![Outgoing::Quit]
                }
            }
            (KeyCode::Char('x'), true) => self.cancel(),
            (KeyCode::Char('d'), true) => {
                self.quit = true;
                vec![Outgoing::Quit]
            }
            (KeyCode::Enter, _) => self.submit(now),
            (KeyCode::Backspace, _) => {
                self.composer.pop();
                Vec::new()
            }
            (KeyCode::Esc, _) => {
                if self.entries.iter().any(|e| matches!(e, Entry::Ask { .. }))
                    && self
                        .task
                        .as_ref()
                        .is_some_and(|t| t.state == a2a::TaskState::InputRequired)
                {
                    self.mode = Mode::Form;
                }
                Vec::new()
            }
            (KeyCode::Up, _) => {
                self.scroll = self.scroll.saturating_sub(1);
                Vec::new()
            }
            (KeyCode::Down, _) => {
                self.scroll = self.scroll.saturating_add(1);
                Vec::new()
            }
            (KeyCode::Char(c), false) => {
                self.composer.push(c);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn submit(&mut self, now: DateTime<Utc>) -> Vec<Outgoing> {
        let text = self.composer.trim().to_owned();
        if text.is_empty() {
            return Vec::new();
        }
        if text == "/model" && !self.opts.models.is_empty() {
            self.composer.clear();
            let selected = self
                .opts
                .models
                .iter()
                .position(|m| m.alias == self.opts.footer.model)
                .unwrap_or(0);
            self.mode = Mode::ModelPicker { selected };
            return Vec::new();
        }
        self.composer.clear();
        let tokens = self.command_tokens(&text);
        self.entries.push(Entry::Turn {
            who: Who::You,
            at: now,
            text: text.clone(),
            tokens,
        });
        let message = self.user_message(vec![a2a::Part::text(text)], HashMap::new());
        self.busy = true;
        vec![Outgoing::Send(Box::new(message))]
    }

    fn cancel(&mut self) -> Vec<Outgoing> {
        self.mode = Mode::Chat;
        match &self.task {
            Some(t) if !t.state.is_terminal() => vec![Outgoing::Cancel(t.id.clone())],
            _ => Vec::new(),
        }
    }

    /// A message from the person: on the open task, or starting a new one that
    /// references the last finished task (R21.7); long-running by default.
    #[must_use]
    pub fn user_message(
        &self,
        parts: Vec<a2a::Part>,
        extra: HashMap<String, Value>,
    ) -> a2a::Message {
        let mut message = a2a::Message::new(a2a::Role::User, parts);
        let mut metadata = self.surface.client_metadata();
        match &self.task {
            Some(t) if !t.state.is_terminal() => {
                message.task_id = Some(t.id.clone());
                message.context_id = Some(t.context_id.clone());
            }
            Some(t) => {
                message.context_id = Some(t.context_id.clone());
                message.reference_task_ids = Some(vec![t.id.clone()]);
                metadata.insert(
                    TASK_EXT.to_owned(),
                    serde_json::json!({ "mode": "long-running" }),
                );
            }
            None => {
                metadata.insert(
                    TASK_EXT.to_owned(),
                    serde_json::json!({ "mode": "long-running" }),
                );
            }
        }
        for (k, v) in extra {
            match (metadata.get_mut(&k), v) {
                (Some(Value::Object(existing)), Value::Object(more)) => existing.extend(more),
                (_, v) => {
                    metadata.insert(k, v);
                }
            }
        }
        message.metadata = Some(metadata);
        message
    }

    fn key_form(&mut self, key: KeyEvent, now: DateTime<Utc>) -> Vec<Outgoing> {
        if key.code == KeyCode::Esc {
            self.mode = Mode::Chat;
            return Vec::new();
        }
        let Some((channel, form)) = self.form_mut() else {
            self.mode = Mode::Chat;
            return Vec::new();
        };
        let channel = channel.clone();
        let Some(action) = form.key(key, now) else {
            return Vec::new();
        };
        form.submitted = Some(action.name.clone());
        let part = a2ui::client_part(&[a2ui::ClientMessage::Action(action)]);
        let mut extra = HashMap::new();
        if !channel.is_empty() {
            extra.insert(
                CHANNELS_EXT.to_owned(),
                serde_json::json!({ "channel": channel, "ask": channel, "reply_to": channel, "question_kind": "a2ui" }),
            );
        }
        let message = self.user_message(vec![part], extra);
        self.mode = Mode::Chat;
        self.busy = true;
        vec![Outgoing::Send(Box::new(message))]
    }

    fn key_picker(&mut self, key: KeyEvent, selected: usize, now: DateTime<Utc>) -> Vec<Outgoing> {
        let len = self.opts.models.len();
        match key.code {
            KeyCode::Esc => self.mode = Mode::Chat,
            KeyCode::Up => {
                self.mode = Mode::ModelPicker {
                    selected: selected.saturating_sub(1),
                };
            }
            KeyCode::Down => {
                self.mode = Mode::ModelPicker {
                    selected: (selected + 1).min(len.saturating_sub(1)),
                };
            }
            KeyCode::Enter => {
                self.mode = Mode::Chat;
                if let Some(entry) = self.opts.models.get(selected) {
                    self.composer = format!("/model {}", entry.alias);
                    return self.submit(now);
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn key_confirm(&mut self, key: KeyEvent, _now: DateTime<Utc>) -> Vec<Outgoing> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                self.mode = Mode::Chat;
                let Some(t) = &self.task else {
                    return Vec::new();
                };
                let mut message = a2a::Message::new(a2a::Role::User, Vec::new());
                message.task_id = Some(t.id.clone());
                message.context_id = Some(t.context_id.clone());
                message.metadata = Some(HashMap::from([(
                    TASK_EXT.to_owned(),
                    serde_json::json!({ "transition": "complete" }),
                )]));
                self.busy = true;
                vec![Outgoing::Send(Box::new(message))]
            }
            KeyCode::Char('x') => self.cancel(),
            KeyCode::Char('n') | KeyCode::Esc => {
                self.mode = Mode::Chat;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Opens the `/model` picker (the composer's `/model` does the same).
    pub fn open_model_picker(&mut self) {
        let selected = self
            .opts
            .models
            .iter()
            .position(|m| m.alias == self.opts.footer.model)
            .unwrap_or(0);
        self.mode = Mode::ModelPicker { selected };
    }

    /// Opens the complete confirmation (`^c` does the same).
    pub fn open_confirm(&mut self) {
        self.mode = Mode::ConfirmComplete;
    }

    /// The label of a model trait.
    #[must_use]
    pub fn trait_label(&self, t: ModelTrait) -> String {
        self.text(t.key())
    }
}

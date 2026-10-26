//! An A2UI basic-catalog form rendered natively in the terminal (R25.4, R27).
//!
//! The form keeps the surface's components and data model, walks the tree from `root`
//! into styled lines, moves focus between its inputs with `tab`, edits them with the
//! arrow keys and typing, and turns an activated button into a client `action` whose
//! context is resolved against the data model. Images and videos render as their alt
//! text, a modal as a box under its trigger, a slider as a bracketed gauge.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use super::view::Theme;
use crate::interaction::renderer::a2ui::{
    A2uiMessage, Action, Children, ChoiceVariant, Component, ComponentKind, Dynamic, IconName,
    TextVariant, UserAction,
};

/// One A2UI surface as the terminal shows it.
#[derive(Debug)]
pub struct Form {
    /// The surface id (`createSurface`).
    pub surface_id: String,
    /// The catalog.
    pub catalog_id: String,
    components: BTreeMap<String, Component>,
    order: Vec<String>,
    data: Value,
    /// The focusable components, in tree order.
    fields: Vec<String>,
    focus: usize,
    cursor: BTreeMap<String, usize>,
    open: BTreeSet<String>,
    tabs: BTreeMap<String, usize>,
    /// The action a button dispatched, once one did.
    pub submitted: Option<String>,
}

impl Form {
    /// The form built from a payload's messages.
    #[must_use]
    pub fn new(messages: &[A2uiMessage]) -> Self {
        let mut form = Self {
            surface_id: String::new(),
            catalog_id: String::new(),
            components: BTreeMap::new(),
            order: Vec::new(),
            data: Value::Object(serde_json::Map::new()),
            fields: Vec::new(),
            focus: 0,
            cursor: BTreeMap::new(),
            open: BTreeSet::new(),
            tabs: BTreeMap::new(),
            submitted: None,
        };
        form.apply(messages);
        form
    }

    /// Applies more messages for the same surface.
    pub fn apply(&mut self, messages: &[A2uiMessage]) {
        for m in messages {
            match m {
                A2uiMessage::CreateSurface(c) => {
                    self.surface_id.clone_from(&c.surface_id);
                    self.catalog_id.clone_from(&c.catalog_id);
                }
                A2uiMessage::UpdateComponents(u) => {
                    for c in &u.components {
                        if !self.components.contains_key(&c.id) {
                            self.order.push(c.id.clone());
                        }
                        self.components.insert(c.id.clone(), c.clone());
                    }
                }
                A2uiMessage::UpdateDataModel(u) => {
                    let path = u.path.clone().unwrap_or_else(|| "/".into());
                    match &u.value {
                        Some(v) if path == "/" || path.is_empty() => self.data = v.clone(),
                        Some(v) => set_pointer(&mut self.data, &path, v.clone()),
                        None => remove_pointer(&mut self.data, &path),
                    }
                }
                A2uiMessage::DeleteSurface(_) => {
                    self.components.clear();
                    self.order.clear();
                }
            }
        }
        self.fields = self.walk_fields();
        if self.focus >= self.fields.len() {
            self.focus = 0;
        }
    }

    /// The root component: `root`, else the first one nothing references.
    #[must_use]
    pub fn root(&self) -> Option<&Component> {
        if let Some(c) = self.components.get("root") {
            return Some(c);
        }
        let referenced: BTreeSet<&str> = self
            .components
            .values()
            .flat_map(|c| children_of(c).into_iter())
            .collect();
        self.order
            .iter()
            .find(|id| !referenced.contains(id.as_str()))
            .and_then(|id| self.components.get(id))
    }

    /// The data model.
    #[must_use]
    pub fn data(&self) -> &Value {
        &self.data
    }

    /// The focused component's id.
    #[must_use]
    pub fn focused(&self) -> Option<&str> {
        self.fields.get(self.focus).map(String::as_str)
    }

    fn walk_fields(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        if let Some(root) = self.root() {
            self.collect_fields(root, &mut seen, &mut out);
        }
        out
    }

    fn collect_fields<'a>(
        &'a self,
        c: &'a Component,
        seen: &mut BTreeSet<&'a str>,
        out: &mut Vec<String>,
    ) {
        if !seen.insert(c.id.as_str()) {
            return;
        }
        if matches!(
            c.kind,
            ComponentKind::Button { .. }
                | ComponentKind::TextField { .. }
                | ComponentKind::CheckBox { .. }
                | ComponentKind::ChoicePicker { .. }
                | ComponentKind::Slider { .. }
                | ComponentKind::DateTimeInput { .. }
        ) {
            out.push(c.id.clone());
        }
        if let ComponentKind::Tabs { tabs } = &c.kind {
            let selected = self.tabs.get(&c.id).copied().unwrap_or(0);
            if let Some(tab) = tabs.get(selected)
                && let Some(child) = self.components.get(&tab.child)
            {
                self.collect_fields(child, seen, out);
            }
            return;
        }
        if let ComponentKind::Modal { trigger, content } = &c.kind {
            if let Some(t) = self.components.get(trigger) {
                self.collect_fields(t, seen, out);
            }
            if self.open.contains(&c.id)
                && let Some(body) = self.components.get(content)
            {
                self.collect_fields(body, seen, out);
            }
            return;
        }
        for id in children_of(c) {
            if let Some(child) = self.components.get(id) {
                self.collect_fields(child, seen, out);
            }
        }
    }

    fn resolve_string(&self, s: &Dynamic<String>) -> String {
        match s {
            Dynamic::Literal(v) => v.clone(),
            Dynamic::Path(p) => self
                .lookup(&p.path)
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                })
                .unwrap_or_default(),
            Dynamic::Call(_) => String::new(),
        }
    }

    fn lookup(&self, path: &str) -> Option<&Value> {
        if path.starts_with('/') {
            self.data.pointer(path)
        } else {
            self.data.pointer(&format!("/{path}"))
        }
    }

    fn write<T>(&mut self, binding: &Dynamic<T>, value: Value) {
        if let Dynamic::Path(p) = binding {
            let path = if p.path.starts_with('/') {
                p.path.clone()
            } else {
                format!("/{}", p.path)
            };
            set_pointer(&mut self.data, &path, value);
        }
    }

    /// Handles a key; a button's activation yields the action to send.
    #[allow(clippy::too_many_lines)]
    pub fn key(&mut self, key: KeyEvent, now: DateTime<Utc>) -> Option<UserAction> {
        match key.code {
            KeyCode::Tab => {
                if !self.fields.is_empty() {
                    self.focus = (self.focus + 1) % self.fields.len();
                }
                return None;
            }
            KeyCode::BackTab => {
                if !self.fields.is_empty() {
                    self.focus = (self.focus + self.fields.len() - 1) % self.fields.len();
                }
                return None;
            }
            _ => {}
        }
        let id = self.focused()?.to_owned();
        let component = self.components.get(&id)?.clone();
        match component.kind {
            ComponentKind::Button { action, .. } => match key.code {
                KeyCode::Enter | KeyCode::Char(' ') => {
                    // A modal's trigger opens the modal rather than acting.
                    if let Some(modal) = self.modal_of(&id) {
                        if self.open.contains(&modal) {
                            self.open.remove(&modal);
                        } else {
                            self.open.insert(modal);
                        }
                        self.fields = self.walk_fields();
                        return None;
                    }
                    self.action(&id, &action, now)
                }
                _ => None,
            },
            ComponentKind::TextField { value, .. } => {
                let mut current = value
                    .as_ref()
                    .map(|v| self.resolve_string(v))
                    .unwrap_or_default();
                match key.code {
                    KeyCode::Char(c) => current.push(c),
                    KeyCode::Backspace => {
                        current.pop();
                    }
                    _ => return None,
                }
                if let Some(binding) = value {
                    self.write(&binding, Value::String(current));
                }
                None
            }
            ComponentKind::DateTimeInput { value, .. } => {
                let mut current = self.resolve_string(&value);
                match key.code {
                    KeyCode::Char(c) => current.push(c),
                    KeyCode::Backspace => {
                        current.pop();
                    }
                    _ => return None,
                }
                self.write(&value, Value::String(current));
                None
            }
            ComponentKind::CheckBox { value, .. } => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
                    let current = match &value {
                        Dynamic::Literal(b) => *b,
                        Dynamic::Path(p) => self
                            .lookup(&p.path)
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        Dynamic::Call(_) => false,
                    };
                    self.write(&value, Value::Bool(!current));
                }
                None
            }
            ComponentKind::ChoicePicker {
                options,
                value,
                variant,
                ..
            } => {
                let cursor = self.cursor.entry(id.clone()).or_insert(0);
                match key.code {
                    KeyCode::Left => *cursor = cursor.saturating_sub(1),
                    KeyCode::Right => *cursor = (*cursor + 1).min(options.len().saturating_sub(1)),
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        let picked = options.get(*cursor).map(|o| o.value.clone())?;
                        let mut selected = self.selected_values(&value);
                        if variant == Some(ChoiceVariant::MultipleSelection) {
                            if let Some(i) = selected.iter().position(|v| *v == picked) {
                                selected.remove(i);
                            } else {
                                selected.push(picked);
                            }
                        } else {
                            selected = vec![picked];
                        }
                        self.write(
                            &value,
                            Value::Array(selected.into_iter().map(Value::String).collect()),
                        );
                    }
                    _ => {}
                }
                None
            }
            ComponentKind::Slider {
                min, max, value, ..
            } => {
                let lo = min.unwrap_or(0.0);
                let step = ((max - lo) / 20.0).max(1.0);
                let current = self.number(&value).unwrap_or(lo);
                let next = match key.code {
                    KeyCode::Left => (current - step).max(lo),
                    KeyCode::Right => (current + step).min(max),
                    _ => return None,
                };
                if let Some(n) = serde_json::Number::from_f64(next) {
                    self.write(&value, Value::Number(n));
                }
                None
            }
            _ => None,
        }
    }

    fn modal_of(&self, trigger: &str) -> Option<String> {
        self.components.values().find_map(|c| match &c.kind {
            ComponentKind::Modal { trigger: t, .. } if t == trigger => Some(c.id.clone()),
            _ => None,
        })
    }

    fn selected_values(&self, value: &Dynamic<Vec<String>>) -> Vec<String> {
        match value {
            Dynamic::Literal(v) => v.clone(),
            Dynamic::Path(p) => self
                .lookup(&p.path)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            Dynamic::Call(_) => Vec::new(),
        }
    }

    fn number(&self, value: &Dynamic<f64>) -> Option<f64> {
        match value {
            Dynamic::Literal(n) => Some(*n),
            Dynamic::Path(p) => self.lookup(&p.path).and_then(Value::as_f64),
            Dynamic::Call(_) => None,
        }
    }

    fn action(&self, source: &str, action: &Action, now: DateTime<Utc>) -> Option<UserAction> {
        let Action::Event(event) = action else {
            return None;
        };
        let context = event
            .event
            .context
            .as_ref()
            .map(|c| {
                c.iter()
                    .map(|(k, v)| {
                        let resolved = match v {
                            Dynamic::Literal(v) => v.clone(),
                            Dynamic::Path(p) => {
                                self.lookup(&p.path).cloned().unwrap_or(Value::Null)
                            }
                            Dynamic::Call(_) => Value::Null,
                        };
                        (k.clone(), resolved)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(UserAction {
            name: event.event.name.clone(),
            surface_id: self.surface_id.clone(),
            source_component_id: source.to_owned(),
            timestamp: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            context,
        })
    }

    /// The form as styled lines, `width` columns wide; `active` means the keys are in it.
    #[must_use]
    pub fn lines(&self, width: usize, theme: Theme, active: bool) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        if let Some(root) = self.root() {
            self.render(root, width, theme, active, &mut seen, &mut out);
        }
        out
    }

    #[allow(clippy::too_many_lines)]
    fn render<'a>(
        &'a self,
        c: &'a Component,
        width: usize,
        theme: Theme,
        active: bool,
        seen: &mut BTreeSet<&'a str>,
        out: &mut Vec<Line<'static>>,
    ) {
        if !seen.insert(c.id.as_str()) {
            return;
        }
        let focused = active && self.focused() == Some(c.id.as_str());
        match &c.kind {
            ComponentKind::Text { text, variant } => {
                let style = match variant {
                    Some(TextVariant::H1 | TextVariant::H2 | TextVariant::H3) => theme.title(),
                    Some(TextVariant::H4 | TextVariant::H5) => theme.bold(),
                    Some(TextVariant::Caption) => theme.dim(),
                    Some(TextVariant::Body) | None => Style::default(),
                };
                for line in wrap(&self.resolve_string(text), width) {
                    out.push(Line::from(Span::styled(line, style)));
                }
            }
            ComponentKind::Image {
                url, description, ..
            }
            | ComponentKind::AudioPlayer { url, description } => {
                let alt = description
                    .as_ref()
                    .map(|d| self.resolve_string(d))
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| self.resolve_string(url));
                out.push(Line::from(Span::styled(format!("[{alt}]"), theme.dim())));
            }
            ComponentKind::Video { url } => {
                out.push(Line::from(Span::styled(
                    format!("[{}]", self.resolve_string(url)),
                    theme.dim(),
                )));
            }
            ComponentKind::Icon { name } => {
                let label = match name {
                    IconName::Named(icon) => serde_json::to_value(icon)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                    IconName::Svg(_) => "svg".into(),
                    IconName::Path(p) => self
                        .lookup(&p.path)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                };
                out.push(Line::from(Span::styled(format!("({label})"), theme.dim())));
            }
            ComponentKind::Row { .. } => {
                let mut spans: Vec<Span<'static>> = Vec::new();
                let mut stacked: Vec<Line<'static>> = Vec::new();
                for id in children_of(c) {
                    let Some(child) = self.components.get(id) else {
                        continue;
                    };
                    let mut lines = Vec::new();
                    self.render(child, width, theme, active, seen, &mut lines);
                    if lines.len() == 1 && stacked.is_empty() {
                        if !spans.is_empty() {
                            spans.push(Span::raw("   "));
                        }
                        spans.extend(lines.remove(0).spans);
                    } else {
                        stacked.extend(lines);
                    }
                }
                if !spans.is_empty() {
                    out.push(Line::from(spans));
                }
                out.extend(stacked);
            }
            ComponentKind::Column { .. } | ComponentKind::List { .. } => {
                let ids = children_of(c);
                for (i, id) in ids.iter().enumerate() {
                    if i > 0 {
                        out.push(Line::default());
                    }
                    if let Some(child) = self.components.get(*id) {
                        self.render(child, width, theme, active, seen, out);
                    }
                }
            }
            ComponentKind::Card { child } => {
                if let Some(child) = self.components.get(child) {
                    self.render(child, width.saturating_sub(2), theme, active, seen, out);
                }
            }
            ComponentKind::Tabs { tabs } => {
                let selected = self.tabs.get(&c.id).copied().unwrap_or(0);
                let mut spans = Vec::new();
                for (i, tab) in tabs.iter().enumerate() {
                    if i > 0 {
                        spans.push(Span::raw("  "));
                    }
                    let title = self.resolve_string(&tab.title);
                    spans.push(if i == selected {
                        Span::styled(title, theme.bold().add_modifier(Modifier::UNDERLINED))
                    } else {
                        Span::styled(title, theme.dim())
                    });
                }
                out.push(Line::from(spans));
                if let Some(tab) = tabs.get(selected)
                    && let Some(child) = self.components.get(&tab.child)
                {
                    self.render(child, width, theme, active, seen, out);
                }
            }
            ComponentKind::Modal { trigger, content } => {
                if let Some(t) = self.components.get(trigger) {
                    self.render(t, width, theme, active, seen, out);
                }
                if self.open.contains(&c.id)
                    && let Some(body) = self.components.get(content)
                {
                    let inner = width.saturating_sub(4);
                    out.push(Line::from(Span::styled(
                        format!("┌{}┐", "─".repeat(inner + 2)),
                        theme.accent(),
                    )));
                    let mut lines = Vec::new();
                    self.render(body, inner, theme, active, seen, &mut lines);
                    for line in lines {
                        let mut spans = vec![Span::styled("│ ", theme.accent())];
                        spans.extend(line.spans);
                        out.push(Line::from(spans));
                    }
                    out.push(Line::from(Span::styled(
                        format!("└{}┘", "─".repeat(inner + 2)),
                        theme.accent(),
                    )));
                }
            }
            ComponentKind::Divider { .. } => {
                out.push(Line::from(Span::styled(
                    "─".repeat(width.min(40)),
                    theme.dim(),
                )));
            }
            ComponentKind::Button { child, variant, .. } => {
                let label = self
                    .components
                    .get(child)
                    .map(|c| self.plain_text(c))
                    .unwrap_or_default();
                let primary = matches!(
                    variant,
                    Some(crate::interaction::renderer::a2ui::ButtonVariant::Primary)
                );
                let text = if primary {
                    format!("  {label}  ")
                } else {
                    format!("[ {label} ]")
                };
                let mut style = if primary {
                    theme.button_primary()
                } else {
                    theme.button()
                };
                if focused {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                out.push(Line::from(Span::styled(text, style)));
            }
            ComponentKind::TextField { label, value, .. } => {
                out.push(Line::from(Span::styled(
                    self.resolve_string(label),
                    theme.bold(),
                )));
                let current = value
                    .as_ref()
                    .map(|v| self.resolve_string(v))
                    .unwrap_or_default();
                let inner = width.saturating_sub(4).max(8);
                let rule = "─".repeat(inner);
                let border = if focused { theme.accent() } else { theme.dim() };
                out.push(Line::from(Span::styled(format!("┌{rule}"), border)));
                let shown = if focused {
                    format!("{current}▏")
                } else {
                    current
                };
                out.push(Line::from(vec![
                    Span::styled("│ ", border),
                    Span::raw(shown),
                ]));
                out.push(Line::from(Span::styled(format!("└{rule}"), border)));
            }
            ComponentKind::CheckBox { label, value, .. } => {
                let checked = match value {
                    Dynamic::Literal(b) => *b,
                    Dynamic::Path(p) => self
                        .lookup(&p.path)
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    Dynamic::Call(_) => false,
                };
                let mut style = Style::default();
                if focused {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                out.push(Line::from(Span::styled(
                    format!(
                        "[{}] {}",
                        if checked { "x" } else { " " },
                        self.resolve_string(label)
                    ),
                    style,
                )));
            }
            ComponentKind::ChoicePicker {
                label,
                options,
                value,
                variant,
                ..
            } => {
                if let Some(label) = label {
                    out.push(Line::from(Span::styled(
                        self.resolve_string(label),
                        theme.bold(),
                    )));
                }
                let selected = self.selected_values(value);
                let cursor = self.cursor.get(&c.id).copied().unwrap_or(0);
                let multiple = *variant == Some(ChoiceVariant::MultipleSelection);
                let mut spans = Vec::new();
                for (i, option) in options.iter().enumerate() {
                    if i > 0 {
                        spans.push(Span::raw("   "));
                    }
                    let on = selected.contains(&option.value);
                    let glyph = match (multiple, on) {
                        (true, true) => "[x]",
                        (true, false) => "[ ]",
                        (false, true) => "(●)",
                        (false, false) => "( )",
                    };
                    let mut style = if on { theme.accent() } else { Style::default() };
                    if focused && i == cursor {
                        style = style.add_modifier(Modifier::REVERSED);
                    }
                    spans.push(Span::styled(
                        format!("{glyph} {}", self.resolve_string(&option.label)),
                        style,
                    ));
                }
                out.push(Line::from(spans));
            }
            ComponentKind::Slider {
                label,
                min,
                max,
                value,
                ..
            } => {
                let lo = min.unwrap_or(0.0);
                let current = self.number(value).unwrap_or(lo);
                let span = (max - lo).max(f64::EPSILON);
                let cells = 20usize;
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss
                )]
                let filled = (((current - lo) / span) * cells as f64)
                    .round()
                    .clamp(0.0, cells as f64) as usize;
                let gauge = format!(
                    "[{}●{}] {current}/{max}",
                    "━".repeat(filled),
                    "─".repeat(cells.saturating_sub(filled))
                );
                let mut spans = Vec::new();
                if let Some(label) = label {
                    spans.push(Span::styled(
                        format!("{} ", self.resolve_string(label)),
                        theme.bold(),
                    ));
                }
                let mut style = theme.accent();
                if focused {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                spans.push(Span::styled(gauge, style));
                out.push(Line::from(spans));
            }
            ComponentKind::DateTimeInput { value, label, .. } => {
                let mut spans = Vec::new();
                if let Some(label) = label {
                    spans.push(Span::styled(
                        format!("{} ", self.resolve_string(label)),
                        theme.bold(),
                    ));
                }
                let current = self.resolve_string(value);
                let shown = if focused {
                    format!("[ {current}▏]")
                } else {
                    format!("[ {current} ]")
                };
                spans.push(Span::styled(
                    shown,
                    if focused { theme.accent() } else { theme.dim() },
                ));
                out.push(Line::from(spans));
            }
        }
    }

    fn plain_text(&self, c: &Component) -> String {
        match &c.kind {
            ComponentKind::Text { text, .. } => self.resolve_string(text),
            ComponentKind::Icon { .. } => String::new(),
            _ => c.id.clone(),
        }
    }
}

/// The ids a component references.
fn children_of(c: &Component) -> Vec<&str> {
    match &c.kind {
        ComponentKind::Row { children, .. }
        | ComponentKind::Column { children, .. }
        | ComponentKind::List { children, .. } => match children {
            Children::Ids(ids) => ids.iter().map(String::as_str).collect(),
            Children::Template(t) => vec![t.component_id.as_str()],
        },
        ComponentKind::Card { child } | ComponentKind::Button { child, .. } => vec![child],
        ComponentKind::Tabs { tabs } => tabs.iter().map(|t| t.child.as_str()).collect(),
        ComponentKind::Modal { trigger, content } => vec![trigger, content],
        _ => Vec::new(),
    }
}

/// Greedy word wrap to `width` columns.
pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

/// Sets `value` at the JSON Pointer `path`, creating objects on the way.
fn set_pointer(root: &mut Value, path: &str, value: Value) {
    let mut node = root;
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    for (i, seg) in segments.iter().enumerate() {
        let last = i + 1 == segments.len();
        if !node.is_object() {
            *node = Value::Object(serde_json::Map::new());
        }
        let map = node.as_object_mut().expect("object");
        if last {
            map.insert((*seg).to_owned(), value);
            return;
        }
        node = map
            .entry((*seg).to_owned())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
    }
}

/// Removes the key at the JSON Pointer `path`.
fn remove_pointer(root: &mut Value, path: &str) {
    let Some((parent, key)) = path.rsplit_once('/') else {
        return;
    };
    let parent = if parent.is_empty() {
        Some(&mut *root)
    } else {
        root.pointer_mut(parent)
    };
    if let Some(Value::Object(map)) = parent {
        map.remove(key);
    }
}

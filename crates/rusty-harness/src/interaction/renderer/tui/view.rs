//! Painting the [`App`] per the prototype: header, conversation, plan strip, composer,
//! key hints; popups for `/model` and the complete confirmation (R27.4, R27.6).

use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui_image::StatefulImage;

use super::app::{App, Entry, Mode, Who};
use crate::assets::{FmtArgs, keys};
use crate::harness::core_loop::plan::StepStatus;
use crate::harness::core_loop::task::SubState;

/// Colours, or none under `NO_COLOR` (NFR accessibility).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    mono: bool,
}

impl Theme {
    /// The terminal's palette.
    #[must_use]
    pub const fn color() -> Self {
        Self { mono: false }
    }

    /// Modifiers only: bold, dim, reversed, underlined.
    #[must_use]
    pub const fn mono() -> Self {
        Self { mono: true }
    }

    /// Monochrome when `NO_COLOR` is set or the terminal is dumb.
    #[must_use]
    pub fn from_env() -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let dumb = std::env::var("TERM").is_ok_and(|t| t == "dumb");
        if no_color || dumb {
            Self::mono()
        } else {
            Self::color()
        }
    }

    /// Whether colours are off.
    #[must_use]
    pub const fn is_mono(self) -> bool {
        self.mono
    }

    fn fg(self, color: Color) -> Style {
        if self.mono {
            Style::default()
        } else {
            Style::default().fg(color)
        }
    }

    /// Secondary text.
    #[must_use]
    pub fn dim(self) -> Style {
        self.fg(Color::DarkGray).add_modifier(Modifier::DIM)
    }
    /// Emphasis.
    #[must_use]
    pub fn bold(self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }
    /// Headings and names.
    #[must_use]
    pub fn title(self) -> Style {
        self.fg(Color::Cyan).add_modifier(Modifier::BOLD)
    }
    /// The accent: commands, selections, borders in focus.
    #[must_use]
    pub fn accent(self) -> Style {
        self.fg(Color::Cyan)
    }
    /// Good news.
    #[must_use]
    pub fn ok(self) -> Style {
        self.fg(Color::Green)
    }
    /// Attention.
    #[must_use]
    pub fn warn(self) -> Style {
        self.fg(Color::Yellow)
    }
    /// Bad news.
    #[must_use]
    pub fn err(self) -> Style {
        self.fg(Color::Red)
    }
    /// A key in the hint bar.
    #[must_use]
    pub fn key(self) -> Style {
        self.bold()
    }
    /// A plain button.
    #[must_use]
    pub fn button(self) -> Style {
        self.accent()
    }
    /// The primary button.
    #[must_use]
    pub fn button_primary(self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        }
    }
}

/// Paints the whole frame.
pub fn draw(app: &mut App, frame: &mut Frame<'_>) {
    draw_with(app, frame, Theme::from_env_or(app.options().mono));
}

impl Theme {
    fn from_env_or(mono: bool) -> Self {
        if mono { Self::mono() } else { Self::from_env() }
    }
}

/// Paints the whole frame with `theme`.
pub fn draw_with(app: &mut App, frame: &mut Frame<'_>, theme: Theme) {
    let area = frame.area();
    let outer = Block::default()
        .borders(Borders::ALL)
        .title_top(header_left(app, theme))
        .title_top(header_right(app, theme).right_aligned());
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(u16::from(!(app.plan.is_empty() && app.task.is_none()))),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .split(inner);

    // One column of breathing room inside the border, as the prototype draws it.
    let pad = |r: Rect| Rect::new(r.x + 1, r.y, r.width.saturating_sub(2), r.height);
    draw_conversation(app, frame, pad(rows[0]), theme);
    if rows[1].height > 0 {
        draw_plan(app, frame, rows[1], theme);
    }
    draw_composer(app, frame, pad(rows[2]), theme);
    draw_hints(app, frame, rows[3], theme);

    match app.mode.clone() {
        Mode::ModelPicker { selected } => draw_model_picker(app, frame, inner, theme, selected),
        Mode::ConfirmComplete => draw_confirm(app, frame, inner, theme),
        Mode::Chat | Mode::Form => {}
    }
}

fn header_left(app: &App, theme: Theme) -> Line<'static> {
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(app.options().agent.clone(), theme.title()),
    ];
    if let Some(task) = &app.task {
        spans.push(Span::styled(
            format!(" · {} · {}", task.id, task.context_id),
            theme.dim(),
        ));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

fn header_right(app: &App, theme: Theme) -> Line<'static> {
    let Some(task) = &app.task else {
        return Line::from(vec![
            Span::styled("○ ", theme.dim()),
            Span::styled(app.text(keys::tui::task::NO_TASK), theme.dim()),
            Span::raw(" "),
        ]);
    };
    let (glyph, style, key) = match task.state {
        a2a::TaskState::Working => ("●", theme.ok(), keys::tui::status::WORKING),
        a2a::TaskState::InputRequired => ("◐", theme.warn(), keys::tui::status::INPUT_REQUIRED),
        a2a::TaskState::Completed => ("✓", theme.ok(), keys::tui::status::COMPLETED),
        a2a::TaskState::Failed => ("✗", theme.err(), keys::tui::status::FAILED),
        a2a::TaskState::Canceled => ("✗", theme.dim(), keys::tui::status::CANCELED),
        a2a::TaskState::Rejected => ("✗", theme.err(), keys::tui::status::REJECTED),
        a2a::TaskState::AuthRequired => ("◐", theme.warn(), keys::tui::status::AUTH_REQUIRED),
        a2a::TaskState::Submitted | a2a::TaskState::Unspecified => {
            ("○", theme.dim(), keys::tui::status::SUBMITTED)
        }
    };
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(format!("{glyph} "), style),
        Span::styled(app.text(key), style),
    ];
    if let Some(sub) = &task.sub_state {
        spans.push(Span::styled(
            format!(" {}", sub_state_label(app, sub)),
            theme.dim(),
        ));
    } else if !app.plan.is_empty() {
        let done = app
            .plan
            .iter()
            .filter(|(_, s)| matches!(s, StepStatus::Done | StepStatus::Skipped))
            .count();
        spans.push(Span::styled(
            format!(
                " {}",
                app.text_with(
                    keys::tui::task::STEP,
                    &FmtArgs::new()
                        .number("done", i64::try_from(done.min(app.plan.len())).unwrap_or(0))
                        .number("total", i64::try_from(app.plan.len()).unwrap_or(0))
                )
            ),
            theme.dim(),
        ));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

fn sub_state_label(app: &App, sub: &SubState) -> String {
    match sub {
        SubState::AwaitingNextTurn => app.text(keys::tui::sub_state::AWAITING_NEXT_TURN),
        SubState::AwaitingParticipant { .. } => {
            app.text(keys::tui::sub_state::AWAITING_PARTICIPANT)
        }
        SubState::Paused => app.text(keys::tui::sub_state::PAUSED),
        SubState::Degraded { .. } => app.text(keys::tui::sub_state::DEGRADED),
        SubState::Compacting => app.text(keys::tui::sub_state::COMPACTING),
        SubState::Delegated { .. } => app.text(keys::tui::sub_state::DELEGATED),
    }
}

fn clock(at: DateTime<Utc>) -> String {
    at.format("%H:%M").to_string()
}

#[allow(clippy::too_many_lines)]
fn draw_conversation(app: &mut App, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let width = usize::from(area.width).saturating_sub(3);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut images: Vec<(usize, usize)> = Vec::new(); // (line index, entry index)
    let agent = app.options().agent.clone();
    let you = app.text(keys::tui::who::YOU);
    let streaming = app.text(keys::tui::STREAMING);
    let asked = |channel: &str| {
        app.text_with(
            keys::tui::a2ui::ASKED,
            &FmtArgs::new().text("channel", channel),
        )
    };
    let form_title = app.text(keys::tui::a2ui::TITLE);
    let in_form = app.mode == Mode::Form;
    let busy = app.busy;
    let last_agent_turn = app.entries.iter().rposition(|e| {
        matches!(
            e,
            Entry::Turn {
                who: Who::Agent,
                ..
            }
        )
    });
    let image_label = |name: &str, alt: &str| {
        app.text_with(
            keys::tui::artifact::IMAGE,
            &FmtArgs::new().text("name", name).text("alt", alt),
        )
    };
    let artifact_label = |name: &str, alt: &str| {
        app.text_with(
            keys::tui::artifact::OTHER,
            &FmtArgs::new().text("name", name).text("alt", alt),
        )
    };
    if app.entries.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            format!("  {}", app.text(keys::tui::prompt::FIRST_MESSAGE)),
            theme.dim(),
        )));
        lines.push(Line::from(Span::styled(
            format!("  {}", app.text(keys::tui::prompt::LONG_RUNNING)),
            theme.dim(),
        )));
    }
    let newest_ask = app
        .entries
        .iter()
        .rposition(|e| matches!(e, Entry::Ask { .. }));
    for (i, entry) in app.entries.iter().enumerate() {
        lines.push(Line::default());
        match entry {
            Entry::Turn {
                who,
                at,
                text,
                tokens,
            } => {
                let name = match who {
                    Who::You => you.clone(),
                    Who::Agent => agent.clone(),
                };
                let mut head = vec![
                    Span::styled(name, theme.title()),
                    Span::styled(format!(" {}", clock(*at)), theme.dim()),
                ];
                if *who == Who::Agent && busy && Some(i) == last_agent_turn {
                    head.push(Span::styled(format!(" · {streaming}"), theme.dim()));
                }
                lines.push(Line::from(head));
                if tokens.is_empty() {
                    let md = tui_markdown::from_str(text);
                    for line in md.lines {
                        let mut spans = vec![Span::raw("  ")];
                        spans.extend(line.spans.into_iter().map(|s| {
                            let style = if theme.is_mono() {
                                Style::default().add_modifier(s.style.add_modifier)
                            } else {
                                s.style
                            };
                            Span::styled(s.content.into_owned(), style)
                        }));
                        lines.push(Line::from(spans));
                    }
                } else {
                    lines.extend(highlighted(text, tokens, theme));
                }
            }
            Entry::Ask { channel, at, form } => {
                lines.push(Line::from(vec![
                    Span::styled(agent.clone(), theme.title()),
                    Span::styled(format!(" {}", clock(*at)), theme.dim()),
                    Span::styled(
                        if channel.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", asked(channel))
                        },
                        theme.dim(),
                    ),
                ]));
                let inner = width.saturating_sub(8);
                let rule = "─".repeat(inner.saturating_sub(form_title.chars().count() + 3));
                let border = if in_form && Some(i) == newest_ask {
                    theme.accent()
                } else {
                    theme.dim()
                };
                lines.push(Line::from(Span::styled(
                    format!("  ╭─ {form_title} {rule}╮"),
                    border,
                )));
                for line in form.lines(
                    inner.saturating_sub(2),
                    theme,
                    in_form && Some(i) == newest_ask,
                ) {
                    let mut spans = vec![Span::styled("  │  ", border)];
                    spans.extend(line.spans);
                    lines.push(Line::from(spans));
                }
                lines.push(Line::from(Span::styled(
                    format!("  ╰{}╯", "─".repeat(inner)),
                    border,
                )));
                if let Some(name) = &form.submitted {
                    lines.push(Line::from(Span::styled(format!("  → {name}"), theme.dim())));
                }
            }
            Entry::Artifact { name, alt, image } => {
                if image.is_some() {
                    lines.push(Line::from(Span::styled(
                        format!("  {}", image_label(name, alt)),
                        theme.dim(),
                    )));
                    images.push((lines.len(), i));
                    for _ in 0..6 {
                        lines.push(Line::default());
                    }
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("  {}", artifact_label(name, alt)),
                        theme.dim(),
                    )));
                }
            }
            Entry::Notice { at, text } => {
                lines.push(Line::from(vec![
                    Span::styled(format!("{} ", clock(*at)), theme.dim()),
                    Span::styled(text.clone(), theme.warn()),
                ]));
            }
        }
    }
    if let Some(task) = &app.task {
        for sub in &task.sub_tasks {
            lines.push(Line::default());
            lines.push(Line::from(vec![Span::styled(
                app.text_with(keys::tui::task::SUB_TASK, &FmtArgs::new().text("task", sub)),
                theme.dim(),
            )]));
        }
    }
    let total = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let fits = area.height;
    // Pin to the bottom when the conversation is longer than the pane; `scroll` is an
    // offset from that end.
    let top = total.saturating_sub(fits).saturating_sub(app.scroll());
    let paragraph = Paragraph::new(Text::from(lines)).scroll((top, 0));
    frame.render_widget(paragraph, area);
    for (line_index, entry_index) in images {
        let y = u16::try_from(line_index).unwrap_or(u16::MAX);
        if y < top || y.saturating_sub(top) + 6 > area.height {
            continue;
        }
        let rect = Rect::new(
            area.x + 2,
            area.y + (y - top),
            area.width.saturating_sub(4).min(40),
            6,
        );
        if let Some(Entry::Artifact {
            image: Some(state), ..
        }) = app.entries.get_mut(entry_index)
        {
            frame.render_stateful_widget(StatefulImage::default(), rect, &mut state.protocol);
        }
    }
}

/// Markdown-free text with its command tokens in the accent colour (R27.6).
fn highlighted(text: &str, tokens: &[(usize, usize)], theme: Theme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut offset = 0;
    for raw in text.split_inclusive('\n') {
        let line = raw.trim_end_matches('\n');
        let mut spans = vec![Span::raw("  ")];
        let mut pos = 0;
        for (start, end) in tokens {
            let (s, e) = (start.saturating_sub(offset), end.saturating_sub(offset));
            if *start < offset || e > line.len() || s < pos {
                continue;
            }
            if s > pos {
                spans.push(Span::raw(line[pos..s].to_owned()));
            }
            spans.push(Span::styled(
                line[s..e].to_owned(),
                theme.accent().add_modifier(Modifier::BOLD),
            ));
            pos = e;
        }
        if pos < line.len() {
            spans.push(Span::raw(line[pos..].to_owned()));
        }
        out.push(Line::from(spans));
        offset += raw.len();
    }
    out
}

fn draw_plan(app: &App, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let mut spans = vec![Span::styled(
        format!(" {} ", app.text(keys::tui::PLAN)),
        theme.dim(),
    )];
    if app.plan.is_empty() {
        spans.push(Span::styled(app.text(keys::tui::plan::NONE), theme.dim()));
    }
    for (name, status) in &app.plan {
        let (glyph, style) = match status {
            StepStatus::Done => ("■", theme.ok()),
            StepStatus::Running => ("▣", theme.accent()),
            StepStatus::Pending => ("□", theme.dim()),
            StepStatus::Skipped => ("◌", theme.dim()),
            StepStatus::Failed => ("✗", theme.err()),
        };
        spans.push(Span::styled(format!("{glyph} "), style));
        spans.push(Span::raw(format!("{name} ")));
    }
    let block = Block::default()
        .borders(Borders::TOP)
        .title_top(Line::from(spans));
    frame.render_widget(block, area);
}

fn draw_composer(app: &App, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let mut lines = Vec::new();
    let in_form = app.mode == Mode::Form;
    let prompt = Span::styled("› ", theme.accent());
    if let Some(since) = app.listening {
        // Push-to-talk is capturing (R27.5): a timer and how to send.
        let elapsed = app.now.map_or(0, |now| (now - since).num_seconds().max(0));
        let key = if app.options().hold {
            keys::tui::voice::LISTENING_HOLD
        } else {
            keys::tui::voice::LISTENING_TOGGLE
        };
        let text = app.text_with(
            key,
            &FmtArgs::new()
                .text("elapsed", format!("{}:{:02}", elapsed / 60, elapsed % 60))
                .text("key", ptt_label(app)),
        );
        lines.push(Line::from(vec![
            Span::styled("● ", theme.err()),
            Span::styled(text, theme.bold()),
        ]));
    } else if in_form {
        lines.push(Line::from(vec![
            prompt,
            Span::styled(app.text(keys::tui::a2ui::ANSWER_HINT), theme.dim()),
        ]));
    } else {
        let tokens = app.command_tokens(&app.composer);
        let mut spans = vec![prompt];
        if tokens.is_empty() {
            spans.push(Span::raw(app.composer.clone()));
        } else {
            spans.extend(
                highlighted(&app.composer, &tokens, theme)
                    .remove(0)
                    .spans
                    .into_iter()
                    .skip(1),
            );
        }
        if app.mode == Mode::Chat {
            spans.push(Span::styled("▏", theme.dim()));
        }
        lines.push(Line::from(spans));
        let matching = app.matching_commands();
        if !matching.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  {}", matching.join(" · ")),
                theme.accent(),
            )));
        }
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn draw_hints(app: &App, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let hint = |key: &str, label: String| -> Vec<Span<'static>> {
        vec![
            Span::styled(format!("  {key}"), theme.key()),
            Span::styled(format!("  {label}"), theme.dim()),
        ]
    };
    let mut spans: Vec<Span<'static>> = Vec::new();
    match app.mode {
        Mode::Form => {
            spans.extend(hint("tab", app.text(keys::tui::hint::NEXT_FIELD)));
            spans.extend(hint("←→", app.text(keys::tui::hint::CHOOSE)));
            spans.extend(hint("enter", app.text(keys::tui::hint::SUBMIT)));
            spans.extend(hint("esc", app.text(keys::tui::hint::BACK)));
        }
        Mode::ModelPicker { .. } => {
            spans.extend(hint("↑↓", app.text(keys::tui::hint::MOVE)));
            spans.extend(hint("enter", app.text(keys::tui::hint::SELECT)));
            spans.extend(hint("esc", app.text(keys::tui::hint::CLOSE)));
        }
        Mode::ConfirmComplete => {
            spans.extend(hint("y", app.text(keys::tui::confirm::YES)));
            spans.extend(hint("n", app.text(keys::tui::confirm::NO)));
            spans.extend(hint("x", app.text(keys::tui::confirm::CANCEL_INSTEAD)));
        }
        Mode::Chat if app.listening.is_some() => {
            let held = if app.options().hold {
                keys::tui::voice::HINT_HOLD
            } else {
                keys::tui::voice::HINT_TOGGLE
            };
            spans.extend(hint(&ptt_label(app), app.text(held)));
            spans.extend(hint("esc", app.text(keys::tui::hint::BACK)));
        }
        Mode::Chat => {
            spans.extend(hint("enter", app.text(keys::tui::hint::SEND)));
            if app.options().voice {
                let talk = if app.options().hold {
                    keys::tui::hint::HOLD_TO_TALK
                } else {
                    keys::tui::hint::PRESS_TO_TALK
                };
                spans.extend(hint(&ptt_label(app), app.text(talk)));
            }
            spans.extend(hint("/", app.text(keys::tui::hint::COMMANDS)));
            spans.extend(hint("^c", app.text(keys::tui::hint::COMPLETE)));
            spans.extend(hint("^x", app.text(keys::tui::hint::CANCEL)));
        }
    }
    let footer = &app.options().footer;
    let mut right = footer.model.clone();
    if let Some(effort) = &footer.effort {
        let _ = write!(right, " · {effort}");
    }
    if !footer.durable.is_empty() {
        let _ = write!(right, " · {}", footer.durable);
    }
    let block = Block::default()
        .borders(Borders::TOP)
        .title_bottom(Line::from(spans))
        .title_bottom(Line::from(Span::styled(format!("{right} "), theme.dim())).right_aligned());
    frame.render_widget(block, area);
}

/// The push-to-talk key as the hints name it.
fn ptt_label(app: &App) -> String {
    match app.options().push_to_talk {
        ' ' => "space".into(),
        c => c.to_string(),
    }
}

fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn draw_model_picker(app: &App, frame: &mut Frame<'_>, area: Rect, theme: Theme, selected: usize) {
    let models = &app.options().models;
    let height = u16::try_from(models.len()).unwrap_or(0) + 6;
    let rect = popup(area, 72, height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.accent())
        .title_top(Line::from(Span::styled(
            format!(" {} ", app.text(keys::tui::model::TITLE)),
            theme.title(),
        )));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let mut lines = vec![
        Line::from(Span::styled(
            app.text(keys::tui::model::SUBTITLE),
            theme.dim(),
        )),
        Line::default(),
    ];
    let current = &app.options().footer.model;
    for (i, m) in models.iter().enumerate() {
        let marker = if i == selected { "▸ " } else { "  " };
        let mut traits: Vec<String> = Vec::new();
        if &m.alias == current {
            traits.push(app.text(keys::tui::model::CURRENT));
        }
        traits.extend(m.traits.iter().map(|t| app.trait_label(*t)));
        let label = format!("{marker}{:<36}", format!("{} ({})", m.label, m.alias));
        let mut style = if i == selected {
            theme.accent().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        if i == selected && theme.is_mono() {
            style = style.add_modifier(Modifier::REVERSED);
        }
        lines.push(Line::from(vec![
            Span::styled(label, style),
            Span::styled(traits.join(" · "), theme.dim()),
        ]));
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("↑↓", theme.key()),
        Span::styled(
            format!("  {}   ", app.text(keys::tui::hint::MOVE)),
            theme.dim(),
        ),
        Span::styled("enter", theme.key()),
        Span::styled(
            format!("  {}   ", app.text(keys::tui::hint::SELECT)),
            theme.dim(),
        ),
        Span::styled("esc", theme.key()),
        Span::styled(
            format!("  {}", app.text(keys::tui::hint::CLOSE)),
            theme.dim(),
        ),
    ]));
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );
}

fn draw_confirm(app: &App, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let sub_tasks = app.task.as_ref().map_or(0, |t| t.sub_tasks.len());
    let body = app.text_with(
        keys::tui::confirm::COMPLETE,
        &FmtArgs::new().number("n", i64::try_from(sub_tasks).unwrap_or(0)),
    );
    let rect = popup(area, 64, 8);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.warn())
        .title_top(Line::from(Span::styled(
            format!(" {} ", app.text(keys::tui::confirm::TITLE)),
            theme.title(),
        )));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let mut lines: Vec<Line<'static>> =
        super::form::wrap(&body, usize::from(inner.width).saturating_sub(2))
            .into_iter()
            .map(|l| Line::from(format!(" {l}")))
            .collect();
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("  y", theme.key()),
        Span::styled(
            format!("  {}    ", app.text(keys::tui::confirm::YES)),
            theme.dim(),
        ),
        Span::styled("n", theme.key()),
        Span::styled(
            format!("  {}    ", app.text(keys::tui::confirm::NO)),
            theme.dim(),
        ),
        Span::styled("x", theme.key()),
        Span::styled(
            format!("  {}", app.text(keys::tui::confirm::CANCEL_INSTEAD)),
            theme.dim(),
        ),
    ]));
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

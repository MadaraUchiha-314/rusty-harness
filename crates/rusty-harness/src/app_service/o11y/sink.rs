//! Two `tracing` layers over the redacting visitor: one forwarding to the host's
//! [`LogSink`] (`wasm32` and embedding hosts, R24.5), one recording for tests.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use super::redact::RedactingVisitor;
use crate::host::{LogRecord, LogSink, SpanRecord};

/// Per-span data kept in the registry's extensions.
#[derive(Debug, Default, Clone)]
struct SpanData {
    fields: BTreeMap<String, String>,
    started: Option<std::time::Instant>,
}

/// Forwards events and finished spans to a [`LogSink`].
pub struct HostSinkLayer {
    sink: Arc<dyn LogSink>,
}

impl core::fmt::Debug for HostSinkLayer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HostSinkLayer").finish_non_exhaustive()
    }
}

impl HostSinkLayer {
    /// A layer writing to `sink`.
    #[must_use]
    pub fn new(sink: Arc<dyn LogSink>) -> Self {
        Self { sink }
    }
}

fn level_name(level: tracing::Level) -> &'static str {
    match level {
        tracing::Level::TRACE => "trace",
        tracing::Level::DEBUG => "debug",
        tracing::Level::INFO => "info",
        tracing::Level::WARN => "warn",
        tracing::Level::ERROR => "error",
    }
}

impl<S> Layer<S> for HostSinkLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut visitor = RedactingVisitor::new(*attrs.metadata().level());
        attrs.record(&mut visitor);
        if let Some(span) = ctx.span(id) {
            let data = SpanData {
                fields: visitor.fields.into_iter().collect(),
                started: Some(std::time::Instant::now()),
            };
            span.extensions_mut().insert(data);
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut visitor = RedactingVisitor::new(*span.metadata().level());
            values.record(&mut visitor);
            if let Some(data) = span.extensions_mut().get_mut::<SpanData>() {
                data.fields.extend(visitor.fields);
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = RedactingVisitor::new(*event.metadata().level());
        event.record(&mut visitor);
        let message = visitor
            .fields
            .iter()
            .find(|(n, _)| n == "message")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        self.sink.emit(LogRecord {
            level: level_name(*event.metadata().level()),
            target: event.metadata().target().to_owned(),
            message,
            fields: visitor
                .fields
                .into_iter()
                .filter(|(n, _)| n != "message")
                .collect(),
        });
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(&id) {
            let data = span
                .extensions()
                .get::<SpanData>()
                .cloned()
                .unwrap_or_default();
            let name = data
                .fields
                .get("point")
                .cloned()
                .unwrap_or_else(|| span.name().to_owned());
            self.sink.export_span(SpanRecord {
                name,
                duration_us: data.started.map_or(0, |s| {
                    u64::try_from(s.elapsed().as_micros()).unwrap_or(u64::MAX)
                }),
                attributes: data.fields.into_iter().collect(),
            });
        }
    }
}

/// A recorded event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedEvent {
    /// The level.
    pub level: &'static str,
    /// The message field.
    pub message: String,
    /// Every other field, redacted.
    pub fields: BTreeMap<String, String>,
}

/// A recorded, closed span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedSpan {
    /// The hook point, or the span's name when it has no `point` field.
    pub point: String,
    /// The enclosing span's point, when nested.
    pub parent: Option<String>,
    /// Every field, redacted.
    pub fields: BTreeMap<String, String>,
}

/// A layer that keeps what it saw, for tests.
#[derive(Debug, Default, Clone)]
pub struct RecordingLayer {
    events: Arc<Mutex<Vec<RecordedEvent>>>,
    spans: Arc<Mutex<Vec<RecordedSpan>>>,
}

impl RecordingLayer {
    /// The events so far.
    #[must_use]
    pub fn events(&self) -> Vec<RecordedEvent> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The closed spans so far, in closing order.
    #[must_use]
    pub fn spans(&self) -> Vec<RecordedSpan> {
        self.spans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl<S> Layer<S> for RecordingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut visitor = RedactingVisitor::new(*attrs.metadata().level());
        attrs.record(&mut visitor);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanData {
                fields: visitor.fields.into_iter().collect(),
                started: None,
            });
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut visitor = RedactingVisitor::new(*span.metadata().level());
            values.record(&mut visitor);
            if let Some(data) = span.extensions_mut().get_mut::<SpanData>() {
                data.fields.extend(visitor.fields);
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = RedactingVisitor::new(*event.metadata().level());
        event.record(&mut visitor);
        let fields: BTreeMap<String, String> = visitor.fields.into_iter().collect();
        let message = fields.get("message").cloned().unwrap_or_default();
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(RecordedEvent {
                level: level_name(*event.metadata().level()),
                message,
                fields: fields.into_iter().filter(|(n, _)| n != "message").collect(),
            });
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(&id) {
            let fields = span
                .extensions()
                .get::<SpanData>()
                .map(|d| d.fields.clone())
                .unwrap_or_default();
            let parent = span.parent().and_then(|p| {
                p.extensions()
                    .get::<SpanData>()
                    .and_then(|d| d.fields.get("point").cloned())
            });
            let point = fields
                .get("point")
                .cloned()
                .unwrap_or_else(|| span.name().to_owned());
            self.spans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(RecordedSpan {
                    point,
                    parent,
                    fields,
                });
        }
    }
}

//! Field redaction for every sink (R24.4): credential-shaped fields are hidden whatever
//! their value, and content fields are truncated above `debug`. `Secret<T>` already
//! prints `[redacted]` by type; this is the second line.

use core::fmt;

use tracing::Level;
use tracing::field::{Field, Visit};
use tracing_subscriber::field::{MakeVisitor, VisitFmt, VisitOutput};
use tracing_subscriber::fmt::format::Writer;

/// The value every credential-shaped field shows.
pub const REDACTED: &str = "[redacted]";

/// Content fields longer than this are truncated above `debug`.
pub const CONTENT_LIMIT: usize = 48;

/// Field names that never show their value.
const SECRET_NAMES: &[&str] = &[
    "key",
    "token",
    "secret",
    "password",
    "authorization",
    "credential",
];

/// Field names that carry user or model content.
const CONTENT_NAMES: &[&str] = &["content", "body", "text", "prompt", "transcript"];

/// Token *counts* (`tokens.in`, `gen_ai.usage.input_tokens`) are usage, not credentials.
const COUNT_NAMES: &[&str] = &["tokens", "token_count", "tokens_"];

fn is_secret_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if COUNT_NAMES.iter().any(|c| lower.contains(c)) && !lower.contains("secret") {
        return SECRET_NAMES
            .iter()
            .filter(|s| **s != "token")
            .any(|s| lower.contains(s));
    }
    SECRET_NAMES.iter().any(|s| lower.contains(s))
}

fn is_content_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    CONTENT_NAMES
        .iter()
        .any(|s| lower == *s || lower.ends_with(&format!("_{s}")))
}

/// The value a sink may show for `name` at `level`.
#[must_use]
pub fn field(name: &str, value: &str, level: Level) -> String {
    if is_secret_name(name) {
        return REDACTED.to_owned();
    }
    if is_content_name(name) && level < Level::DEBUG && value.chars().count() > CONTENT_LIMIT {
        let kept: String = value.chars().take(CONTENT_LIMIT).collect();
        return format!(
            "{kept}… [{} chars truncated]",
            value.chars().count() - CONTENT_LIMIT
        );
    }
    value.to_owned()
}

/// A visitor that collects redacted `(name, value)` pairs.
#[derive(Debug, Default)]
pub struct RedactingVisitor {
    level: Option<Level>,
    /// The pairs seen so far.
    pub fields: Vec<(String, String)>,
}

impl RedactingVisitor {
    /// A visitor for an event or span at `level`.
    #[must_use]
    pub fn new(level: Level) -> Self {
        Self {
            level: Some(level),
            fields: Vec::new(),
        }
    }

    fn push(&mut self, name: &str, value: &str) {
        let level = self.level.unwrap_or(Level::INFO);
        self.fields
            .push((name.to_owned(), field(name, value, level)));
    }
}

impl Visit for RedactingVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.push(field.name(), &format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field.name(), value);
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field.name(), &value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field.name(), &value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field.name(), &value.to_string());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.push(field.name(), &value.to_string());
    }
}

/// The field formatter for the console layer: the same redaction, written inline.
#[derive(Debug, Default, Clone, Copy)]
pub struct RedactingFields;

/// Writes redacted fields as `name=value` pairs.
pub struct RedactingWriter<'a> {
    writer: Writer<'a>,
    result: fmt::Result,
    first: bool,
}

impl fmt::Debug for RedactingWriter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedactingWriter").finish_non_exhaustive()
    }
}

impl<'a> MakeVisitor<Writer<'a>> for RedactingFields {
    type Visitor = RedactingWriter<'a>;

    fn make_visitor(&self, target: Writer<'a>) -> Self::Visitor {
        RedactingWriter {
            writer: target,
            result: Ok(()),
            first: true,
        }
    }
}

impl RedactingWriter<'_> {
    fn write(&mut self, name: &str, value: &str) {
        if self.result.is_err() {
            return;
        }
        let shown = field(name, value, Level::INFO);
        let sep = if self.first { "" } else { " " };
        self.first = false;
        self.result = if name == "message" {
            write!(self.writer, "{sep}{shown}")
        } else {
            write!(self.writer, "{sep}{name}={shown}")
        };
    }
}

impl Visit for RedactingWriter<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.write(field.name(), &format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.write(field.name(), value);
    }
}

impl VisitOutput<fmt::Result> for RedactingWriter<'_> {
    fn finish(self) -> fmt::Result {
        self.result
    }
}

impl VisitFmt for RedactingWriter<'_> {
    fn writer(&mut self) -> &mut dyn fmt::Write {
        &mut self.writer
    }
}

#[cfg(test)]
mod tests {
    use super::{REDACTED, field};
    use tracing::Level;

    #[test]
    fn credential_shaped_fields_hide_and_content_truncates_above_debug() {
        assert_eq!(field("api_key", "sk-1", Level::ERROR), REDACTED);
        assert_eq!(field("Authorization", "Bearer x", Level::INFO), REDACTED);
        let long = "x".repeat(100);
        assert!(field("content", &long, Level::INFO).contains("truncated"));
        assert_eq!(field("content", &long, Level::DEBUG), long);
        assert_eq!(field("message_text", "short", Level::INFO), "short");
        assert_eq!(field("task", "task_1", Level::INFO), "task_1");
        assert_eq!(field("tokens.in", "10", Level::INFO), "10", "a count");
        assert_eq!(field("gen_ai.usage.input_tokens", "10", Level::INFO), "10");
        assert_eq!(
            field("tokens_key", "x", Level::INFO),
            REDACTED,
            "still a key"
        );
        assert_eq!(field("refresh_token", "x", Level::INFO), REDACTED);
    }
}

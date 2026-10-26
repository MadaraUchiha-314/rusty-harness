//! W3C trace context for the one-trace-per-task rule (R24.1).
//!
//! A task adopts a `traceparent` when it is created: the caller's, when the request
//! carried a valid one, else the current span's (when an OpenTelemetry layer is active),
//! else a fresh one. It is persisted on the task record and on every inbox event, so the
//! spans of every later turn, signal and worker join the same trace. [`adopt`] makes a
//! root span a child of it in the exported trace; without an OpenTelemetry layer it is a
//! no-op and the `traceparent` field on the span still names the trace.

use core::fmt;

/// The request header.
pub const HEADER: &str = "traceparent";

/// A parsed `traceparent` (version 00).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceParent {
    /// The trace id.
    pub trace_id: [u8; 16],
    /// The parent span id.
    pub span_id: [u8; 8],
    /// The sampled flag.
    pub sampled: bool,
}

fn hex_bytes<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; N];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

impl TraceParent {
    /// Parses `00-<trace id>-<span id>-<flags>`; `None` for anything else, including
    /// the all-zero ids the specification forbids.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.trim().split('-');
        let version = parts.next()?;
        if version.len() != 2 || version.eq_ignore_ascii_case("ff") {
            return None;
        }
        let trace_id = hex_bytes::<16>(parts.next()?)?;
        let span_id = hex_bytes::<8>(parts.next()?)?;
        let flags = u8::from_str_radix(parts.next()?, 16).ok()?;
        if trace_id == [0; 16] || span_id == [0; 8] {
            return None;
        }
        Some(Self {
            trace_id,
            span_id,
            sampled: flags & 1 == 1,
        })
    }

    /// A fresh, sampled trace.
    #[must_use]
    pub fn mint() -> Self {
        let trace_id = uuid::Uuid::new_v4().into_bytes();
        let mut span_id = [0u8; 8];
        span_id.copy_from_slice(&uuid::Uuid::new_v4().into_bytes()[..8]);
        Self {
            trace_id,
            span_id,
            sampled: true,
        }
    }

    /// The trace id as 32 hex characters.
    #[must_use]
    pub fn trace_id_hex(&self) -> String {
        hex(&self.trace_id)
    }
}

impl fmt::Display for TraceParent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "00-{}-{}-{:02x}",
            hex(&self.trace_id),
            hex(&self.span_id),
            u8::from(self.sampled)
        )
    }
}

/// The trace id named by a `traceparent`, when it parses.
#[must_use]
pub fn trace_id(traceparent: &str) -> Option<String> {
    TraceParent::parse(traceparent).map(|t| t.trace_id_hex())
}

/// What a request brings in: `header` when valid, else the current span's, else nothing
/// (the task's own, persisted one then applies, or a fresh one for a new task).
#[must_use]
pub fn incoming(header: Option<&str>) -> Option<String> {
    header
        .and_then(TraceParent::parse)
        .map(|t| t.to_string())
        .or_else(current)
}

/// The `traceparent` a task adopts: `header` when valid, else the current span's, else
/// a fresh one.
#[must_use]
pub fn for_task(header: Option<&str>) -> String {
    header
        .and_then(TraceParent::parse)
        .map(|t| t.to_string())
        .or_else(current)
        .unwrap_or_else(|| TraceParent::mint().to_string())
}

/// The current span's `traceparent`, when an OpenTelemetry layer is active and the span
/// is part of a valid trace.
#[must_use]
pub fn current() -> Option<String> {
    #[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
    {
        use opentelemetry::trace::TraceContextExt as _;
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        let context = tracing::Span::current().context();
        let span = context.span();
        let sc = span.span_context();
        if sc.is_valid() {
            return Some(
                TraceParent {
                    trace_id: sc.trace_id().to_bytes(),
                    span_id: sc.span_id().to_bytes(),
                    sampled: sc.is_sampled(),
                }
                .to_string(),
            );
        }
    }
    None
}

/// Makes `span` a child of `traceparent` in the exported trace. A no-op when the value
/// does not parse, without an OpenTelemetry layer, and on `wasm32`.
pub fn adopt(span: &tracing::Span, traceparent: &str) {
    let Some(parent) = TraceParent::parse(traceparent) else {
        return;
    };
    #[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
    {
        use opentelemetry::trace::{
            SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
        };
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        let flags = if parent.sampled {
            TraceFlags::SAMPLED
        } else {
            TraceFlags::default()
        };
        let remote = SpanContext::new(
            TraceId::from_bytes(parent.trace_id),
            SpanId::from_bytes(parent.span_id),
            flags,
            true,
            TraceState::default(),
        );
        // `Err` only when the span is disabled; nothing to join then.
        let _ = span.set_parent(opentelemetry::Context::new().with_remote_span_context(remote));
    }
    #[cfg(not(all(feature = "rt-tokio", not(target_arch = "wasm32"))))]
    {
        let _ = (span, parent);
    }
}

#[cfg(test)]
mod tests {
    use super::{TraceParent, for_task, trace_id};

    #[test]
    fn a_traceparent_round_trips_and_invalid_ones_are_refused() {
        let s = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
        let t = TraceParent::parse(s).expect("valid");
        assert_eq!(t.to_string(), s);
        assert!(t.sampled);
        assert_eq!(
            trace_id(s).as_deref(),
            Some("0af7651916cd43dd8448eb211c80319c")
        );
        for bad in [
            "",
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331",
            "ff-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
            "00-00000000000000000000000000000000-b7ad6b7169203331-01",
            "00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01",
            "00-zz-b7ad6b7169203331-01",
        ] {
            assert!(TraceParent::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_task_adopts_the_callers_traceparent_or_mints_one() {
        let s = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
        assert_eq!(for_task(Some(s)), s);
        let minted = for_task(Some("garbage"));
        assert!(TraceParent::parse(&minted).is_some(), "{minted}");
        assert_ne!(for_task(None), for_task(None), "fresh ids every time");
        assert_ne!(TraceParent::mint().trace_id, [0; 16]);
    }
}

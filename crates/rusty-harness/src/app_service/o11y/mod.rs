//! Observability as hooks: tracing spans on every catalogue point, OTLP export, the
//! redaction layer (R24).
//!
//! [`TracingHooks::install`] puts an `in` hook on an operation that wraps `next` in a
//! span named after the point, carrying the task id, context id, the operation's target
//! and whatever attributes the operation reports; nested operations become child spans
//! and a failure closes the span with its error. The dispatcher itself logs vetoes and
//! pre-dispatch failures, so nothing is lost before the span opens. [`init`] installs the
//! subscriber: a console layer, an OTLP layer when `[o11y] otlp_endpoint` is set
//! (native), and the host's [`LogSink`] layer when the host supplies one (`wasm32`).
//! Every layer sees fields through [`redact`], which hides credential-shaped fields and
//! truncates content fields above `debug` (R24.4).

use std::sync::Arc;

use tracing::Instrument;

use crate::config::O11yConfig;
use crate::error::HarnessError;
use crate::harness::hooks::{Hook, HookCx, HookDispatcher, HookOrder, InHook, Next, Operation};
use crate::harness_async_trait;
use crate::host::{Host, LogSink};
use crate::registry::EntityId;

pub mod redact;
mod sink;

pub use sink::{HostSinkLayer, RecordingLayer};

/// The `in` hook that spans an operation (R24.1, R4.8).
#[derive(Debug, Default, Clone, Copy)]
pub struct TracingHooks;

impl TracingHooks {
    /// Installs the span hook on `O`. Registered last-outermost (priority `i32::MIN`) so
    /// every other `in` hook runs inside the span.
    pub fn install<O: Operation>(dispatcher: &HookDispatcher) -> EntityId
    where
        O::Input: Sync,
        O::Output: Sync,
    {
        let id = EntityId::new(format!("o11y.tracing.{}", O::POINT.name()));
        dispatcher.register::<O>(
            id.clone(),
            Hook::In(Arc::new(TracingHooks)),
            HookOrder {
                priority: i32::MIN,
                after: Vec::new(),
            },
        );
        id
    }
}

harness_async_trait! {
    impl<O: Operation> InHook<O> for TracingHooks
    where
        O::Input: Sync,
        O::Output: Sync,
    {
        async fn run<'a>(&self, cx: &'a HookCx, input: O::Input, next: Next<'a, O>) -> Result<O::Output, HarnessError> {
            let span = tracing::info_span!(
                "hook",
                point = O::POINT.name(),
                task = cx.task.as_deref().unwrap_or(""),
                context_id = cx.context_id.as_deref().unwrap_or(""),
                traceparent = cx.traceparent.as_deref().unwrap_or(""),
                target = %O::identity(&input),
                outcome = tracing::field::Empty,
                error = tracing::field::Empty,
            );
            let result = next(input.clone()).instrument(span.clone()).await;
            match &result {
                Ok(output) => {
                    span.record("outcome", "ok");
                    for (key, value) in O::attributes(&input, output) {
                        tracing::info!(parent: &span, attribute = key, value = %value, "attribute");
                    }
                }
                Err(err) => {
                    span.record("outcome", "error");
                    span.record("error", tracing::field::display(err));
                }
            }
            result
        }
    }
}

/// Keeps the subscriber alive; drop it to stop exporting.
pub struct Guard {
    _default: Option<tracing::subscriber::DefaultGuard>,
    #[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
    _provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl core::fmt::Debug for Guard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("o11y::Guard").finish_non_exhaustive()
    }
}

/// How [`init`] installs the subscriber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// For the whole process (the binary): fails if one is already set.
    Global,
    /// For the current thread until the guard drops (tests, embedded use).
    Thread,
}

/// An observability setup failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum O11yError {
    /// A global subscriber is already installed.
    #[error("a tracing subscriber is already installed")]
    AlreadyInstalled,
    /// The OTLP exporter could not be built.
    #[error("otlp exporter: {0}")]
    Exporter(String),
    /// The log level does not parse.
    #[error("log level {0:?} is not a valid filter")]
    Level(String),
}

/// Installs the subscriber per `config` (R24.2, R24.4, R24.5).
///
/// # Errors
///
/// [`O11yError`] when the level does not parse, the exporter cannot be built, or a
/// global subscriber already exists.
pub fn init(config: &O11yConfig, host: &Host, scope: Scope) -> Result<Guard, O11yError> {
    use tracing_subscriber::layer::SubscriberExt;

    let filter = tracing_subscriber::EnvFilter::try_new(&config.log_level)
        .map_err(|_| O11yError::Level(config.log_level.clone()))?;
    let sink_layer = host
        .log_sink()
        .ok()
        .map(|sink: Arc<dyn LogSink>| HostSinkLayer::new(sink));
    let console_layer = if sink_layer.is_some() {
        None
    } else {
        Some(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .fmt_fields(redact::RedactingFields),
        )
    };
    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(console_layer)
        .with(sink_layer);

    #[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
    let (otlp_layer, provider) = match &config.otlp_endpoint {
        Some(endpoint) => {
            use opentelemetry::trace::TracerProvider as _;
            use opentelemetry_otlp::WithExportConfig as _;
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(endpoint.as_str())
                .build()
                .map_err(|e| O11yError::Exporter(e.to_string()))?;
            let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .build();
            let tracer = provider.tracer("rusty-harness");
            (
                Some(tracing_opentelemetry::layer().with_tracer(tracer)),
                Some(provider),
            )
        }
        None => (None, None),
    };
    #[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
    let registry = registry.with(otlp_layer);

    let guard = match scope {
        Scope::Global => {
            tracing::subscriber::set_global_default(registry)
                .map_err(|_| O11yError::AlreadyInstalled)?;
            None
        }
        Scope::Thread => Some(tracing::subscriber::set_default(registry)),
    };
    Ok(Guard {
        _default: guard,
        #[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
        _provider: provider,
    })
}

#[cfg(test)]
mod tests {
    use super::{RecordingLayer, TracingHooks, redact};
    use crate::harness::hooks::{HookCx, HookDispatcher, HookPoint, Operation};
    use crate::host::{Host, Secret};
    use crate::registry::Registry;
    use std::sync::Arc;
    use tracing_subscriber::layer::SubscriberExt;

    struct Call;
    impl Operation for Call {
        const POINT: HookPoint = HookPoint::ModelCall;
        type Input = String;
        type Output = String;
        fn identity(input: &String) -> String {
            input.clone()
        }
        fn attributes(_input: &String, output: &String) -> Vec<(&'static str, String)> {
            vec![("tokens.out", output.len().to_string())]
        }
    }

    struct Tool;
    impl Operation for Tool {
        const POINT: HookPoint = HookPoint::ToolExecute;
        type Input = String;
        type Output = String;
    }

    fn cx() -> HookCx {
        let mut cx = HookCx::new(Arc::new(Registry::new()), Arc::new(Host::testing()));
        cx.task = Some("task_9".into());
        cx
    }

    #[tokio::test]
    async fn every_operation_opens_a_span_with_its_attributes_and_nests() {
        let recording = RecordingLayer::default();
        let subscriber = tracing_subscriber::registry().with(recording.clone());
        let _guard = tracing::subscriber::set_default(subscriber);
        let d = Arc::new(HookDispatcher::new());
        TracingHooks::install::<Call>(&d);
        TracingHooks::install::<Tool>(&d);
        let inner = Arc::clone(&d);
        let out = d
            .run::<Call, _, _>(&cx(), "gpt".into(), move |input| async move {
                let tool =
                    inner
                        .run::<Tool, _, _>(&cx(), "echo".into(), |i| async move {
                            Ok(format!("ran {i}"))
                        })
                        .await?;
                Ok(format!("{input}: {tool}"))
            })
            .await
            .expect("runs");
        assert_eq!(out, "gpt: ran echo");
        let spans = recording.spans();
        let names: Vec<_> = spans.iter().map(|s| s.point.clone()).collect();
        assert_eq!(names, ["tool.execute", "model.call"]);
        assert_eq!(
            spans[0].parent.as_deref(),
            Some("model.call"),
            "the tool span nests under the model span"
        );
        assert!(
            spans
                .iter()
                .all(|s| s.fields.get("task").map(String::as_str) == Some("task_9"))
        );
        assert_eq!(
            spans[1].fields.get("outcome").map(String::as_str),
            Some("ok")
        );
        assert!(
            recording
                .events()
                .iter()
                .any(|e| e.fields.get("attribute").map(String::as_str) == Some("tokens.out"))
        );
    }

    #[tokio::test]
    async fn a_failing_operation_closes_its_span_with_the_error() {
        let recording = RecordingLayer::default();
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(recording.clone()),
        );
        let d = HookDispatcher::new();
        TracingHooks::install::<Tool>(&d);
        let err = d
            .run::<Tool, _, _>(&cx(), "x".into(), |_| async {
                Err(crate::HarnessError::Internal("disk".into()))
            })
            .await;
        assert!(err.is_err());
        let span = &recording.spans()[0];
        assert_eq!(
            span.fields.get("outcome").map(String::as_str),
            Some("error")
        );
        assert!(span.fields.get("error").is_some_and(|e| e.contains("disk")));
    }

    #[tokio::test]
    async fn abuse_canary_credential_appears_nowhere_observable() {
        // Abuse case 7, first leg: logs and traces. A credential in a `Secret`, a raw
        // field with a credential-shaped name, and a credential inside a content field
        // above debug never reach a sink.
        let recording = RecordingLayer::default();
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(recording.clone()),
        );
        let secret = Secret::new(String::from("sk-canary-0xdeadbeef"));
        tracing::info!(api_key = "sk-canary-raw", key = ?secret, "model call");
        tracing::info!(
            content = %format!("{} sk-canary-in-content", "x".repeat(60)),
            "delivered"
        );
        tracing::debug!(content = "sk-canary-debug-level", "debug content");
        let d = HookDispatcher::new();
        TracingHooks::install::<Tool>(&d);
        let _ = d
            .run::<Tool, _, _>(
                &cx(),
                "sk-canary-as-target".into(),
                |i| async move { Ok(i) },
            )
            .await;
        let everything = format!("{:?} {:?}", recording.events(), recording.spans());
        assert!(
            !everything.contains("sk-canary-0xdeadbeef"),
            "Secret printed"
        );
        assert!(
            !everything.contains("sk-canary-raw"),
            "credential-shaped field printed: {everything}"
        );
        assert!(
            !everything.contains("sk-canary-in-content"),
            "content above debug printed: {everything}"
        );
        assert!(
            everything.contains("sk-canary-debug-level"),
            "debug-level content is kept"
        );
        assert!(everything.contains("[redacted]"));
        assert_eq!(
            redact::field("authorization", "Bearer x", tracing::Level::INFO),
            "[redacted]"
        );
    }
}

//! Langfuse as a hook set on the public catalogue, not core code (R24.3).
//!
//! Langfuse ingests OTLP and reads its observation model from span attributes. These
//! hooks open one child span inside the catalogue span of three operations and record
//! those attributes on it: `task.run` becomes an `agent` observation carrying the
//! session (the A2A context id) and the trace name, `model.call` a `generation` with
//! the model, the token usage and the finish reason, `tool.execute` a `tool` with the
//! tool's name. Enabled by `[o11y] langfuse = true`; everything else (export, redaction,
//! the task trace) is the ordinary tracing setup.

use std::sync::Arc;

use tracing::Instrument;
use tracing::field::Empty;

use crate::error::HarnessError;
use crate::harness::core_loop::agent::{ModelCall, TaskRun};
use crate::harness::hooks::{Hook, HookCx, HookDispatcher, HookOrder, InHook, Next};
use crate::harness::tools::ToolExecute;
use crate::harness_async_trait;
use crate::registry::EntityId;

/// Inside the catalogue span ([`super::TracingHooks`] registers at `i32::MIN`).
const ORDER: HookOrder = HookOrder {
    priority: i32::MIN + 1,
    after: Vec::new(),
};

/// The three Langfuse hooks.
#[derive(Debug, Default, Clone, Copy)]
pub struct LangfuseHooks;

impl LangfuseHooks {
    /// Installs the agent, generation and tool observations.
    pub fn install(dispatcher: &HookDispatcher) -> Vec<EntityId> {
        let agent = EntityId::new("o11y.langfuse.agent");
        let generation = EntityId::new("o11y.langfuse.generation");
        let tool = EntityId::new("o11y.langfuse.tool");
        dispatcher.register::<TaskRun>(agent.clone(), Hook::In(Arc::new(AgentObservation)), ORDER);
        dispatcher.register::<ModelCall>(
            generation.clone(),
            Hook::In(Arc::new(GenerationObservation)),
            ORDER,
        );
        dispatcher.register::<ToolExecute>(
            tool.clone(),
            Hook::In(Arc::new(ToolObservation)),
            ORDER,
        );
        vec![agent, generation, tool]
    }
}

/// `task.run` as a Langfuse `agent` observation.
#[derive(Debug)]
struct AgentObservation;

harness_async_trait! {
    impl InHook<TaskRun> for AgentObservation {
        async fn run<'a>(&self, cx: &'a HookCx, input: <TaskRun as crate::harness::hooks::Operation>::Input, next: Next<'a, TaskRun>) -> Result<<TaskRun as crate::harness::hooks::Operation>::Output, HarnessError> {
            let span = tracing::info_span!(
                "agent",
                langfuse.observation.type = "agent",
                langfuse.trace.name = cx.task.as_deref().unwrap_or("task"),
                langfuse.session.id = cx.context_id.as_deref().unwrap_or(""),
                langfuse.observation.level = Empty,
                langfuse.observation.status_message = Empty,
            );
            let result = next(input).instrument(span.clone()).await;
            if let Err(err) = &result {
                span.record("langfuse.observation.level", "ERROR");
                span.record("langfuse.observation.status_message", tracing::field::display(err));
            }
            result
        }
    }
}

/// `model.call` as a Langfuse `generation`.
#[derive(Debug)]
struct GenerationObservation;

harness_async_trait! {
    impl InHook<ModelCall> for GenerationObservation {
        async fn run<'a>(&self, cx: &'a HookCx, input: <ModelCall as crate::harness::hooks::Operation>::Input, next: Next<'a, ModelCall>) -> Result<<ModelCall as crate::harness::hooks::Operation>::Output, HarnessError> {
            let span = tracing::info_span!(
                "generation",
                langfuse.observation.type = "generation",
                langfuse.session.id = cx.context_id.as_deref().unwrap_or(""),
                gen_ai.request.model = input.model.as_deref().unwrap_or(""),
                gen_ai.usage.input_tokens = Empty,
                gen_ai.usage.output_tokens = Empty,
                gen_ai.usage.total_tokens = Empty,
                langfuse.observation.usage_details = Empty,
                gen_ai.response.finish_reasons = Empty,
                langfuse.observation.level = Empty,
                langfuse.observation.status_message = Empty,
            );
            let result = next(input).instrument(span.clone()).await;
            match &result {
                Ok(turn) => {
                    if let Some(usage) = &turn.usage {
                        span.record("gen_ai.usage.input_tokens", usage.input_tokens);
                        span.record("gen_ai.usage.output_tokens", usage.output_tokens);
                        span.record("gen_ai.usage.total_tokens", usage.input_tokens + usage.output_tokens);
                        span.record(
                            "langfuse.observation.usage_details",
                            format!(
                                "{{\"input\":{},\"output\":{},\"cache_read_input_tokens\":{}}}",
                                usage.input_tokens, usage.output_tokens, usage.cached_input_tokens
                            ),
                        );
                    }
                    if let Some(stop) = &turn.stop {
                        span.record("gen_ai.response.finish_reasons", tracing::field::debug(stop));
                    }
                }
                Err(err) => {
                    span.record("langfuse.observation.level", "ERROR");
                    span.record("langfuse.observation.status_message", tracing::field::display(err));
                }
            }
            result
        }
    }
}

/// `tool.execute` as a Langfuse `tool` observation.
#[derive(Debug)]
struct ToolObservation;

harness_async_trait! {
    impl InHook<ToolExecute> for ToolObservation {
        async fn run<'a>(&self, cx: &'a HookCx, input: <ToolExecute as crate::harness::hooks::Operation>::Input, next: Next<'a, ToolExecute>) -> Result<<ToolExecute as crate::harness::hooks::Operation>::Output, HarnessError> {
            let span = tracing::info_span!(
                "tool",
                langfuse.observation.type = "tool",
                langfuse.session.id = cx.context_id.as_deref().unwrap_or(""),
                gen_ai.tool.name = %input.call.name,
                langfuse.observation.level = Empty,
                langfuse.observation.status_message = Empty,
            );
            let result = next(input).instrument(span.clone()).await;
            match &result {
                Ok(Err(tool_err)) => {
                    span.record("langfuse.observation.level", "ERROR");
                    span.record("langfuse.observation.status_message", tracing::field::display(tool_err));
                }
                Err(err) => {
                    span.record("langfuse.observation.level", "ERROR");
                    span.record("langfuse.observation.status_message", tracing::field::display(err));
                }
                Ok(Ok(_)) => {}
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LangfuseHooks;
    use crate::app_service::o11y::{RecordingLayer, TracingHooks};
    use crate::assets::Prompt;
    use crate::harness::core_loop::agent::ModelCall;
    use crate::harness::hooks::{HookCx, HookDispatcher};
    use crate::harness::models::llm::{ModelRequest, ModelTurn, StopReason, Usage};
    use crate::host::Host;
    use crate::registry::Registry;
    use std::sync::Arc;
    use tracing_subscriber::layer::SubscriberExt;

    #[tokio::test]
    async fn a_model_call_becomes_a_generation_inside_the_catalogue_span() {
        let recording = RecordingLayer::default();
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(recording.clone()),
        );
        let d = HookDispatcher::new();
        TracingHooks::install::<ModelCall>(&d);
        LangfuseHooks::install(&d);
        let mut cx = HookCx::new(Arc::new(Registry::new()), Arc::new(Host::testing()));
        cx.task = Some("t1".into());
        cx.context_id = Some("ctx-7".into());
        let mut request = ModelRequest::new(Prompt::default(), vec![]);
        request.model = Some("gpt-6.1".into());
        let turn = ModelTurn {
            text: "hi".into(),
            tool_calls: vec![],
            usage: Some(Usage {
                input_tokens: 10,
                output_tokens: 4,
                cached_input_tokens: 8,
            }),
            stop: Some(StopReason::EndTurn),
            latency: core::time::Duration::from_millis(5),
        };
        let out = d
            .run::<ModelCall, _, _>(&cx, request, move |_| async move { Ok(turn) })
            .await
            .expect("runs");
        assert_eq!(out.text, "hi");
        let spans = recording.spans();
        let generation = spans
            .iter()
            .find(|s| s.point == "generation")
            .expect("a generation span");
        assert_eq!(generation.parent.as_deref(), Some("model.call"));
        let f = &generation.fields;
        assert_eq!(
            f.get("langfuse.observation.type").map(String::as_str),
            Some("generation")
        );
        assert_eq!(
            f.get("langfuse.session.id").map(String::as_str),
            Some("ctx-7")
        );
        assert_eq!(
            f.get("gen_ai.request.model").map(String::as_str),
            Some("gpt-6.1")
        );
        assert_eq!(
            f.get("gen_ai.usage.input_tokens").map(String::as_str),
            Some("10")
        );
        assert_eq!(
            f.get("gen_ai.usage.output_tokens").map(String::as_str),
            Some("4")
        );
        assert_eq!(
            f.get("gen_ai.usage.total_tokens").map(String::as_str),
            Some("14")
        );
        assert!(
            f.get("langfuse.observation.usage_details")
                .is_some_and(|u| u.contains("\"cache_read_input_tokens\":8"))
        );
        assert!(
            f.get("gen_ai.response.finish_reasons")
                .is_some_and(|r| r.contains("EndTurn"))
        );
        assert!(
            f.get("langfuse.observation.level").is_none(),
            "no error level on success"
        );
    }
}

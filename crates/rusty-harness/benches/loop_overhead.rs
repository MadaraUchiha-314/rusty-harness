//! The harness's own cost per turn (T7, NFR performance; target p50 < 5 ms): one
//! `send_message` through the whole loop (task open, context build, the model call,
//! delivery, the store commit) against a model that answers at once, so what is measured
//! is the harness and nothing else.
//!
//! `cargo bench -p rusty_harness --bench loop_overhead`; the median is the p50.

use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use futures::StreamExt as _;
use rusty_harness::config::Config;
use rusty_harness::harness::models::llm::{
    EventStream, Llm, Modality, ModelCapabilities, ModelError, ModelEvent, ModelRequest, StopReason,
};
use rusty_harness::host::Host;
use rusty_harness::registry::{Entity, EntityId, EntityKind};
use rusty_harness::{HarnessBuilder, harness_async_trait};

/// Answers every call with the same short reply, immediately.
#[derive(Debug)]
struct Instant {
    id: EntityId,
    caps: ModelCapabilities,
}

impl Entity for Instant {
    fn kind(&self) -> EntityKind {
        EntityKind::Llm
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Llm for Instant {
        fn capabilities(&self) -> &ModelCapabilities {
            &self.caps
        }

        async fn stream(&self, _request: ModelRequest) -> Result<EventStream<'static>, ModelError> {
            Ok(rusty_harness::runtime::boxed_stream(futures::stream::iter([
                Ok(ModelEvent::TextDelta { text: "Done.".into() }),
                Ok(ModelEvent::Stop { reason: StopReason::EndTurn }),
            ])))
        }
    }
}

fn loop_overhead(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let llm = Arc::new(Instant {
        id: EntityId::new("default"),
        caps: ModelCapabilities {
            input_modalities: [Modality::Text].into(),
            tools: true,
            caching: false,
            max_window_tokens: 128_000,
            effort_levels: vec![],
        },
    });
    let harness = rt.block_on(async {
        HarnessBuilder::new(Config::default())
            .host(Host::testing())
            .llm(llm)
            .build()
            .await
            .expect("builds")
    });
    let mut n = 0u64;
    c.bench_function("one turn through the loop (instant model)", |b| {
        b.to_async(&rt).iter(|| {
            n += 1;
            let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("say done")]);
            message.task_id = Some(format!("bench-{n}"));
            message.context_id = Some("bench".into());
            let request = a2a::SendMessageRequest {
                message,
                configuration: None,
                metadata: None,
                tenant: None,
            };
            let harness = &harness;
            async move {
                let events = harness.send_message(request, None).count().await;
                assert!(events > 0);
            }
        });
    });
}

criterion_group!(benches, loop_overhead);
criterion_main!(benches);

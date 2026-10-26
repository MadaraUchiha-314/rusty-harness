//! The prompt-cache hit rate on a short conversation against a real provider (T7, R15.2):
//! every call shares the stable prefix the context positions put first (system prompt,
//! extensions, tool definitions), so from the second call on the provider should serve
//! it from its cache.
//!
//! ```sh
//! cargo run -p rusty_harness --features rt-tokio --example cache_hit_rate -- \
//!     [model, default gpt-6.1-sol] [turns, default 4]
//! ```
//!
//! The key is the reference `env:OPENAI_API_KEY`; it is resolved by the host, never read
//! here. `RH_CACHE_SYSTEM_PROMPT=file:<path>` swaps in a longer system prompt, for a prefix
//! above the provider's caching floor. Prints one row per call and the overall hit rate.

use std::sync::{Arc, Mutex};

use futures::StreamExt as _;
use rusty_harness::config::{Config, ModelConfig, ModelProvider};
use rusty_harness::harness::models::llm::openai::OpenAiLlm;
use rusty_harness::harness::models::llm::{
    EventStream, Llm, ModelCapabilities, ModelError, ModelEvent, ModelRequest, Usage,
};
use rusty_harness::host::Host;
use rusty_harness::registry::{Entity, EntityId, EntityKind};
use rusty_harness::{HarnessBuilder, harness_async_trait};

/// Forwards to the provider and keeps each call's usage.
struct Tap {
    inner: Arc<dyn Llm>,
    usage: Arc<Mutex<Vec<Usage>>>,
}

impl Entity for Tap {
    fn kind(&self) -> EntityKind {
        EntityKind::Llm
    }
    fn entity_id(&self) -> &EntityId {
        self.inner.entity_id()
    }
}

harness_async_trait! {
    impl Llm for Tap {
        fn capabilities(&self) -> &ModelCapabilities {
            self.inner.capabilities()
        }

        async fn stream(&self, request: ModelRequest) -> Result<EventStream<'static>, ModelError> {
            let usage = Arc::clone(&self.usage);
            let events = self.inner.stream(request).await?;
            Ok(rusty_harness::runtime::boxed_stream(events.inspect(move |e| {
                if let Ok(ModelEvent::Usage { usage: u }) = e {
                    usage.lock().expect("usage").push(*u);
                }
            })))
        }
    }
}

const TURNS: [&str; 4] = [
    "In one sentence: what is an agent harness?",
    "In one sentence: why does a harness keep a durable inbox?",
    "In one sentence: what does a hook in the prompt pipeline do?",
    "In one sentence: why cache the stable prefix of a prompt?",
];

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let model = args.next().unwrap_or_else(|| "gpt-6.1-sol".into());
    let turns: usize = args
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(TURNS.len());
    let host = Host::native();
    let model_config = ModelConfig {
        provider: ModelProvider::Openai,
        model: model.clone(),
        api_key: "env:OPENAI_API_KEY".parse().expect("a reference"),
        base_url: None,
        effort: None,
        retry: rusty_harness::config::RetryConfig::default(),
        api: None,
    };
    let provider = OpenAiLlm::from_config("default", &model_config, &host)
        .await
        .expect("the OpenAI model (is OPENAI_API_KEY set?)");
    let usage = Arc::new(Mutex::new(Vec::new()));
    let tap = Arc::new(Tap {
        inner: Arc::new(provider),
        usage: Arc::clone(&usage),
    });
    // A stable prefix above the provider's caching floor (OpenAI caches from 1024 tokens):
    // `RH_CACHE_SYSTEM_PROMPT=file:<path>` replaces the default system prompt.
    let mut config = Config::default();
    config.agent.system_prompt = std::env::var("RH_CACHE_SYSTEM_PROMPT").ok();
    let harness = HarnessBuilder::new(config)
        .host(host)
        .llm(tap)
        .build()
        .await
        .expect("builds");
    for (i, text) in TURNS.iter().cycle().take(turns).enumerate() {
        let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text(*text)]);
        message.task_id = Some(format!("cache-{i}"));
        message.context_id = Some("cache-hit-rate".into());
        let request = a2a::SendMessageRequest {
            message,
            configuration: None,
            metadata: None,
            tenant: None,
        };
        let events: Vec<_> = harness.send_message(request, None).collect().await;
        for event in events {
            match event {
                Ok(a2a::StreamResponse::StatusUpdate(u))
                    if u.status.state == a2a::TaskState::Failed =>
                {
                    let why = u
                        .status
                        .message
                        .as_ref()
                        .and_then(a2a::Message::text)
                        .unwrap_or_default();
                    panic!("turn {i} failed: {why}");
                }
                Err(e) => panic!("turn {i}: {e}"),
                Ok(_) => {}
            }
        }
    }
    let usage = usage.lock().expect("usage").clone();
    println!("model: {model}\n\n| Call | Input tokens | Cached | Hit rate |\n|---|---|---|---|");
    for (i, u) in usage.iter().enumerate() {
        println!(
            "| {} | {} | {} | {:.0}% |",
            i + 1,
            u.input_tokens,
            u.cached_input_tokens,
            rate(u.cached_input_tokens, u.input_tokens)
        );
    }
    let input: u64 = usage.iter().map(|u| u.input_tokens).sum();
    let cached: u64 = usage.iter().map(|u| u.cached_input_tokens).sum();
    let later_input: u64 = usage.iter().skip(1).map(|u| u.input_tokens).sum();
    let later_cached: u64 = usage.iter().skip(1).map(|u| u.cached_input_tokens).sum();
    println!(
        "\nOverall: {cached} of {input} input tokens cached ({:.0}%); calls 2–{}: {:.0}%.",
        rate(cached, input),
        usage.len(),
        rate(later_cached, later_input)
    );
}

#[allow(clippy::cast_precision_loss)] // token counts are far below 2^52
fn rate(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

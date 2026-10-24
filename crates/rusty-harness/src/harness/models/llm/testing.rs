//! A model that replays recorded turns, for every test that drives the loop without a
//! provider.

use std::collections::BTreeSet;
use std::sync::Mutex;

use super::{EventStream, Llm, Modality, ModelCapabilities, ModelError, ModelEvent, ModelRequest};
use crate::harness_async_trait;
use crate::registry::{Entity, EntityId, EntityKind};

/// Replays one recorded event sequence per call, in order; records every request.
#[derive(Debug)]
pub struct RecordedLlm {
    id: EntityId,
    caps: ModelCapabilities,
    turns: Mutex<std::collections::VecDeque<Vec<ModelEvent>>>,
    requests: Mutex<Vec<ModelRequest>>,
    failures: Mutex<u32>,
    retryable: bool,
    attempts: Mutex<u32>,
}

impl RecordedLlm {
    /// A model answering each call with the next sequence of `turns`.
    #[must_use]
    pub fn new(id: &str, turns: Vec<Vec<ModelEvent>>) -> Self {
        Self {
            id: EntityId::new(id),
            caps: ModelCapabilities {
                input_modalities: BTreeSet::from([Modality::Text, Modality::Image]),
                tools: true,
                caching: false,
                max_window_tokens: 128_000,
                effort_levels: vec![],
            },
            turns: Mutex::new(turns.into()),
            requests: Mutex::new(Vec::new()),
            failures: Mutex::new(0),
            retryable: true,
            attempts: Mutex::new(0),
        }
    }

    /// A model that fails `failures` times (retryably or not), then answers with `turn`.
    #[must_use]
    pub fn failing_then(id: &str, failures: u32, retryable: bool, turn: Vec<ModelEvent>) -> Self {
        let mut llm = Self::new(id, vec![turn]);
        llm.failures = Mutex::new(failures);
        llm.retryable = retryable;
        llm
    }

    /// Replaces the capabilities.
    #[must_use]
    pub fn with_capabilities(mut self, caps: ModelCapabilities) -> Self {
        self.caps = caps;
        self
    }

    /// Every request seen so far.
    #[must_use]
    pub fn requests(&self) -> Vec<ModelRequest> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// How many calls were made, failures included.
    #[must_use]
    pub fn attempts(&self) -> u32 {
        *self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Parses a recorded fixture: a JSON array of turns, each an array of events.
    ///
    /// # Errors
    ///
    /// When the JSON does not have that shape.
    pub fn from_json(id: &str, json: &str) -> Result<Self, serde_json::Error> {
        Ok(Self::new(id, serde_json::from_str(json)?))
    }
}

impl Entity for RecordedLlm {
    fn kind(&self) -> EntityKind {
        EntityKind::Llm
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Llm for RecordedLlm {
        fn capabilities(&self) -> &ModelCapabilities {
            &self.caps
        }

        async fn stream(&self, request: ModelRequest) -> Result<EventStream<'static>, ModelError> {
            *self.attempts.lock().unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            super::check_modalities(&self.caps, &request)?;
            self.requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(request);
            {
                let mut failures = self.failures.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if *failures > 0 {
                    *failures -= 1;
                    return Err(if self.retryable {
                        ModelError::Retryable { status: Some(503), message: "recorded failure".into(), retry_after: None }
                    } else {
                        ModelError::NonRetryable { status: Some(401), message: "recorded failure".into() }
                    });
                }
            }
            let turn = self
                .turns
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_front()
                .ok_or_else(|| ModelError::NonRetryable { status: None, message: "no recorded turn left".into() })?;
            Ok(crate::runtime::boxed_stream(futures::stream::iter(turn.into_iter().map(Ok))))
        }
    }
}

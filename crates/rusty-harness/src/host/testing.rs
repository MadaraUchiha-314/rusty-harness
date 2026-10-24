//! Deterministic host capabilities for tests: a canned HTTP client, a fixed clock, a
//! seeded random source, a map of secrets, an in-memory file system and a log sink that
//! records what it saw. Compiled on every target so the browser test suite can use them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use chrono::{DateTime, TimeZone, Utc};
use futures::stream;

use super::{
    Clock, FileSystem, Host, HostError, HttpClient, HttpRequest, HttpResponse,
    HttpStreamingResponse, LogRecord, LogSink, Rng, Secret, SecretError, SecretRef, SecretResolver,
    SpanRecord,
};
use crate::harness_async_trait;

/// A responder the canned client calls per request.
pub type Responder = dyn Fn(&HttpRequest) -> Result<HttpResponse, HostError> + Send + Sync;

/// An HTTP client that answers from a responder and records every request.
pub struct MockHttp {
    responder: Box<Responder>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl core::fmt::Debug for MockHttp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MockHttp")
            .field("requests", &self.requests().len())
            .finish()
    }
}

impl MockHttp {
    /// A client that answers every request through `responder`.
    pub fn new(
        responder: impl Fn(&HttpRequest) -> Result<HttpResponse, HostError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            responder: Box::new(responder),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// A client that answers every request with `status` and `body`.
    #[must_use]
    pub fn always(status: http::StatusCode, body: impl Into<Bytes>) -> Self {
        let body = body.into();
        Self::new(move |_| {
            Ok(HttpResponse {
                status,
                headers: http::HeaderMap::new(),
                body: body.clone(),
            })
        })
    }

    /// Every request seen so far, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

harness_async_trait! {
    impl HttpClient for MockHttp {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HostError> {
            let response = (self.responder)(&request);
            self.requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(request);
            response
        }

        async fn send_streaming(&self, request: HttpRequest) -> Result<HttpStreamingResponse, HostError> {
            let response = self.send(request).await?;
            Ok(HttpStreamingResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(stream::once(async move { Ok(response.body) })),
            })
        }
    }
}

/// A clock that returns one instant until told otherwise.
#[derive(Debug)]
pub struct FixedClock {
    now: Mutex<DateTime<Utc>>,
}

impl Default for FixedClock {
    fn default() -> Self {
        Self::at(
            Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, 0)
                .single()
                .expect("valid"),
        )
    }
}

impl FixedClock {
    /// A clock stopped at `now`.
    #[must_use]
    pub fn at(now: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }

    /// Moves the clock forward.
    pub fn advance(&self, by: chrono::Duration) {
        let mut now = self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *now += by;
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// A deterministic random source (xorshift64*), seeded.
#[derive(Debug)]
pub struct SeededRng {
    state: Mutex<u64>,
}

impl SeededRng {
    /// A source whose sequence is fixed by `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            state: Mutex::new(seed.max(1)),
        }
    }
}

impl Rng for SeededRng {
    fn fill(&self, buf: &mut [u8]) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for chunk in buf.chunks_mut(8) {
            let mut x = *state;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            *state = x;
            let word = x.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes();
            chunk.copy_from_slice(&word[..chunk.len()]);
        }
    }
}

/// Secrets from a map; anything else is unresolvable, by name.
#[derive(Debug, Default)]
pub struct MapSecrets {
    values: BTreeMap<SecretRef, String>,
}

impl MapSecrets {
    /// A resolver that knows `values`.
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = (SecretRef, String)>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }
}

harness_async_trait! {
    impl SecretResolver for MapSecrets {
        async fn resolve(&self, reference: &SecretRef) -> Result<Secret<String>, SecretError> {
            self.values.get(reference).map(|v| Secret::new(v.clone())).ok_or_else(|| SecretError::Unresolvable {
                reference: reference.to_string(),
                reason: "not in the test map".into(),
            })
        }
    }
}

/// An in-memory file system.
#[derive(Debug, Default)]
pub struct MemFs {
    files: Mutex<BTreeMap<PathBuf, Bytes>>,
}

impl MemFs {
    /// A file system holding `files`.
    #[must_use]
    pub fn new(files: impl IntoIterator<Item = (PathBuf, Bytes)>) -> Self {
        Self {
            files: Mutex::new(files.into_iter().collect()),
        }
    }
}

harness_async_trait! {
    impl FileSystem for MemFs {
        async fn read(&self, path: &Path) -> Result<Bytes, HostError> {
            self.files
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(path)
                .cloned()
                .ok_or_else(|| HostError::Io(format!("{}: not found", path.display())))
        }

        async fn write_atomic(&self, path: &Path, contents: Bytes) -> Result<(), HostError> {
            self.files.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(path.to_path_buf(), contents);
            Ok(())
        }

        async fn list(&self, dir: &Path) -> Result<Vec<PathBuf>, HostError> {
            Ok(self
                .files
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .keys()
                .filter(|p| p.parent() == Some(dir))
                .cloned()
                .collect())
        }
    }
}

/// A sink that keeps every record it was given.
#[derive(Debug, Default)]
pub struct RecordingSink {
    logs: Mutex<Vec<LogRecord>>,
    spans: Mutex<Vec<SpanRecord>>,
}

impl RecordingSink {
    /// The log lines so far.
    #[must_use]
    pub fn logs(&self) -> Vec<LogRecord> {
        self.logs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The spans so far.
    #[must_use]
    pub fn spans(&self) -> Vec<SpanRecord> {
        self.spans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl LogSink for RecordingSink {
    fn emit(&self, record: LogRecord) {
        self.logs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(record);
    }

    fn export_span(&self, span: SpanRecord) {
        self.spans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(span);
    }
}

impl Host {
    /// A host for tests: a fixed clock, a seeded random source, an empty secret map, an
    /// empty in-memory file system, a recording sink, and an HTTP client that answers
    /// `404` to everything. Replace any of them through [`Host::builder`].
    #[must_use]
    pub fn testing() -> Host {
        Host::builder()
            .http(Arc::new(MockHttp::always(
                http::StatusCode::NOT_FOUND,
                Bytes::new(),
            )))
            .clock(Arc::new(FixedClock::default()))
            .rng(Arc::new(SeededRng::new(42)))
            .secrets(Arc::new(MapSecrets::default()))
            .fs(Arc::new(MemFs::default()))
            .log_sink(Arc::new(RecordingSink::default()))
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::{MapSecrets, SeededRng};
    use crate::host::{Rng, SecretRef, SecretResolver};

    #[test]
    fn the_seeded_rng_is_deterministic() {
        let (a, b) = (SeededRng::new(9), SeededRng::new(9));
        let (mut x, mut y) = ([0u8; 11], [0u8; 11]);
        a.fill(&mut x);
        b.fill(&mut y);
        assert_eq!(x, y);
        assert_ne!(x, [0u8; 11]);
    }

    #[test]
    fn an_unknown_reference_fails_by_name_never_by_value() {
        let secrets = MapSecrets::new([(SecretRef::Env("A".into()), "sk-canary-9".into())]);
        let err = futures::executor::block_on(secrets.resolve(&SecretRef::Env("B".into())))
            .expect_err("missing");
        assert!(err.to_string().contains("env:B"));
        assert!(!err.to_string().contains("canary"));
    }
}

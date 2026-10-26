//! What A2A 1.0 defines and the SDK does not yet implement (R19.9), in one marked
//! module: `A2A-Extensions` negotiation and echo (R19.3), the authenticated caller
//! (R19.4, Q10) and the per-caller limits (abuse 8). Everything here is a 1.0 feature
//! the SDK lacks, never an older-protocol behaviour; each item is tracked upstream
//! (`docs/specs/issue-3/evidence/upstream-issues.md`) and goes when the SDK ships it.
//!
//! The pieces run as one axum middleware ([`gate`]) in front of the SDK's routers. The
//! outcome of a request travels to the handlers in reserved service parameters
//! ([`CALLER_PARAM`], [`ACTIVE_EXTENSIONS_PARAM`]), because the SDK builds
//! `ServiceParams` from every request header and sets `ExecutorContext.user` to `None`.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use a2a_server::ServiceParams;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::config::{LimitsConfig, SecuritySchemeConfig};
use crate::host::{Host, Secret};

/// The service parameter the SDK fills from the `A2A-Extensions` header.
pub const EXTENSIONS_PARAM: &str = "a2a-extensions";
/// The reserved key the negotiator writes the active set to.
pub const ACTIVE_EXTENSIONS_PARAM: &str = "x-rusty-harness-active-extensions";
/// The reserved key the authenticator writes the caller's id to.
pub const CALLER_PARAM: &str = "x-rusty-harness-caller";
/// The header a caller sets to act as a named participant (one credential, many people
/// on one deployment); without it the caller is the credential's principal.
pub const PARTICIPANT_HEADER: &str = "x-rusty-harness-participant";
/// The principal of the configured credential, and of a loopback caller.
pub const DEFAULT_PRINCIPAL: &str = "operator";

/// JSON-RPC error codes for what the protocol expresses over HTTP status alone.
pub mod error_code {
    /// No or bad credentials (HTTP 401).
    pub const AUTHENTICATION_REQUIRED: i32 = -32010;
    /// Credentials present, operation not allowed (HTTP 403).
    pub const AUTHORIZATION: i32 = -32011;
    /// The caller's token bucket is empty (HTTP 429).
    pub const RATE_LIMITED: i32 = -32012;
    /// The caller holds `streams_per_caller` streams already (HTTP 429).
    pub const TOO_MANY_STREAMS: i32 = -32013;
}

/// The extensions a request named, as the header lists them (comma separated).
#[must_use]
pub fn requested_extensions(params: &ServiceParams) -> Vec<String> {
    split_list(params.get(EXTENSIONS_PARAM))
}

/// The active set: what the negotiator wrote, else what the request named.
#[must_use]
pub fn active_extensions(params: &ServiceParams) -> Vec<String> {
    match params.get(ACTIVE_EXTENSIONS_PARAM) {
        Some(values) => split_list(Some(values)),
        None => requested_extensions(params),
    }
}

/// The authenticated caller's id, when the gate set one.
#[must_use]
pub fn caller_id(params: &ServiceParams) -> Option<String> {
    params.get(CALLER_PARAM).and_then(|v| v.first()).cloned()
}

fn split_list(values: Option<&Vec<String>>) -> Vec<String> {
    values
        .into_iter()
        .flatten()
        .flat_map(|v| v.split(','))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Intersects what a request names with what the card advertises (R19.3).
#[derive(Debug, Clone)]
pub struct ExtensionNegotiator {
    advertised: BTreeSet<String>,
}

impl ExtensionNegotiator {
    /// A negotiator for the card's extensions.
    #[must_use]
    pub fn new(advertised: impl IntoIterator<Item = String>) -> Self {
        Self {
            advertised: advertised.into_iter().collect(),
        }
    }

    /// The active set: the requested extensions the card advertises, in request order,
    /// without duplicates.
    #[must_use]
    pub fn negotiate(&self, requested: &[String]) -> Vec<String> {
        let mut active = Vec::new();
        for uri in requested {
            if self.advertised.contains(uri) && !active.contains(uri) {
                active.push(uri.clone());
            }
        }
        active
    }
}

/// `true` when `a` and `b` are equal, in time that depends on their lengths only.
#[must_use]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = u8::from(a.len() != b.len());
    let longest = a.len().max(b.len());
    for i in 0..longest {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= x ^ y;
    }
    diff == 0
}

/// The credential a deployment requires (Q10): a bearer token or an API key, resolved
/// from a `SecretRef` once at startup and never logged.
pub enum Authenticator {
    /// Loopback only: no credential, the caller is [`DEFAULT_PRINCIPAL`].
    None,
    /// `Authorization: Bearer <token>`.
    Bearer(Secret<String>),
    /// `<header>: <key>`.
    ApiKey {
        /// The header name, lowercase.
        header: String,
        /// The key.
        key: Secret<String>,
    },
}

impl core::fmt::Debug for Authenticator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::None => "Authenticator::None",
            Self::Bearer(_) => "Authenticator::Bearer",
            Self::ApiKey { .. } => "Authenticator::ApiKey",
        })
    }
}

/// Why the gate refused a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateError {
    /// No or bad credentials.
    Unauthenticated,
    /// Too many requests.
    RateLimited,
}

impl GateError {
    /// The A2A error and the HTTP status.
    #[must_use]
    pub fn to_a2a(&self) -> (axum::http::StatusCode, a2a::A2AError) {
        match self {
            Self::Unauthenticated => (
                axum::http::StatusCode::UNAUTHORIZED,
                a2a::A2AError::new(
                    error_code::AUTHENTICATION_REQUIRED,
                    "authentication required",
                ),
            ),
            Self::RateLimited => (
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                a2a::A2AError::new(error_code::RATE_LIMITED, "rate limited"),
            ),
        }
    }
}

impl Authenticator {
    /// From `[server.security]`, resolving the secret through the host.
    ///
    /// # Errors
    ///
    /// The secret resolver's error, by name.
    pub async fn from_config(
        security: Option<&SecuritySchemeConfig>,
        host: &Host,
    ) -> Result<Self, crate::HarnessError> {
        Ok(match security {
            None => Self::None,
            Some(SecuritySchemeConfig::Bearer { token }) => {
                Self::Bearer(resolve(host, token).await?)
            }
            Some(SecuritySchemeConfig::ApiKey { header, key }) => Self::ApiKey {
                header: header.to_ascii_lowercase(),
                key: resolve(host, key).await?,
            },
        })
    }

    /// The caller's principal for `headers`, or why not (boundary 1).
    ///
    /// # Errors
    ///
    /// [`GateError::Unauthenticated`].
    pub fn authenticate(&self, headers: &axum::http::HeaderMap) -> Result<String, GateError> {
        let presented = match self {
            Self::None => None,
            Self::Bearer(token) => {
                let value = headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| {
                        v.strip_prefix("Bearer ")
                            .or_else(|| v.strip_prefix("bearer "))
                    })
                    .unwrap_or("");
                Some(constant_time_eq(
                    value.trim().as_bytes(),
                    token.expose().as_bytes(),
                ))
            }
            Self::ApiKey { header, key } => {
                let value = headers
                    .get(header.as_str())
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                Some(constant_time_eq(
                    value.trim().as_bytes(),
                    key.expose().as_bytes(),
                ))
            }
        };
        if presented == Some(false) {
            return Err(GateError::Unauthenticated);
        }
        let participant = headers
            .get(PARTICIPANT_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|p| !p.is_empty() && p.len() <= 128);
        Ok(participant.map_or_else(|| DEFAULT_PRINCIPAL.to_owned(), str::to_owned))
    }
}

async fn resolve(
    host: &Host,
    reference: &crate::host::SecretRef,
) -> Result<Secret<String>, crate::HarnessError> {
    let secrets = host.secrets()?;
    secrets
        .resolve(reference)
        .await
        .map_err(|e| crate::HarnessError::Internal(e.to_string()))
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    refilled: Instant,
}

/// A token bucket per caller: `requests_per_minute` capacity, refilled continuously
/// (abuse 8; inline, no rate-limit crate per design.md § minimalism).
#[derive(Debug)]
pub struct RateLimiter {
    per_minute: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl RateLimiter {
    /// A limiter allowing `per_minute` requests per caller.
    #[must_use]
    pub fn new(per_minute: u32) -> Self {
        Self {
            per_minute: f64::from(per_minute.max(1)),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Takes one token for `caller` at `now`; `false` when the bucket is empty.
    pub fn allow(&self, caller: &str, now: Instant) -> bool {
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        let bucket = buckets.entry(caller.to_owned()).or_insert(Bucket {
            tokens: self.per_minute,
            refilled: now,
        });
        let elapsed = now.saturating_duration_since(bucket.refilled).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.per_minute / 60.0).min(self.per_minute);
        bucket.refilled = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// How many streams each caller holds open (abuse 8).
#[derive(Debug)]
pub struct StreamBudget {
    per_caller: u32,
    open: Mutex<HashMap<String, u32>>,
}

/// One held stream slot; released on drop.
#[derive(Debug)]
pub struct StreamSlot {
    budget: Arc<StreamBudget>,
    caller: String,
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        let mut open = self
            .budget
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(n) = open.get_mut(&self.caller) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                open.remove(&self.caller);
            }
        }
    }
}

impl StreamBudget {
    /// A budget of `per_caller` streams.
    #[must_use]
    pub fn new(per_caller: u32) -> Self {
        Self {
            per_caller: per_caller.max(1),
            open: Mutex::new(HashMap::new()),
        }
    }

    /// Takes a slot for `caller`, or `None` when the caller is at the limit.
    #[must_use]
    pub fn acquire(self: &Arc<Self>, caller: &str) -> Option<StreamSlot> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let n = open.entry(caller.to_owned()).or_insert(0);
        if *n >= self.per_caller {
            return None;
        }
        *n += 1;
        Some(StreamSlot {
            budget: Arc::clone(self),
            caller: caller.to_owned(),
        })
    }

    /// How many streams `caller` holds.
    #[must_use]
    pub fn held(&self, caller: &str) -> u32 {
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(caller)
            .copied()
            .unwrap_or(0)
    }
}

/// The gate's state: shared by the middleware and the guarded handler.
#[derive(Debug)]
pub struct Gate {
    /// Who may call.
    pub authenticator: Authenticator,
    /// What the card advertises.
    pub negotiator: ExtensionNegotiator,
    /// Requests per caller.
    pub rate: RateLimiter,
    /// Streams per caller.
    pub streams: Arc<StreamBudget>,
}

impl Gate {
    /// A gate for `[server.security]`, `[server.limits]` and the advertised extensions.
    ///
    /// # Errors
    ///
    /// The secret resolver's error.
    pub async fn new(
        security: Option<&SecuritySchemeConfig>,
        limits: &LimitsConfig,
        advertised: impl IntoIterator<Item = String>,
        host: &Host,
    ) -> Result<Self, crate::HarnessError> {
        Ok(Self {
            authenticator: Authenticator::from_config(security, host).await?,
            negotiator: ExtensionNegotiator::new(advertised),
            rate: RateLimiter::new(limits.requests_per_minute),
            streams: Arc::new(StreamBudget::new(limits.streams_per_caller)),
        })
    }
}

fn refuse(error: &GateError) -> Response {
    let (status, a2a) = error.to_a2a();
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": a2a.to_jsonrpc_error() });
    let mut response = (status, axum::Json(body)).into_response();
    if *error == GateError::Unauthenticated {
        response.headers_mut().insert(
            axum::http::header::WWW_AUTHENTICATE,
            axum::http::HeaderValue::from_static("Bearer"),
        );
    }
    response
}

/// The middleware (design.md § A2A API server): authenticate, rate-limit, negotiate
/// extensions, hand the caller and the active set to the handlers, echo the active set.
/// The agent card is public.
pub async fn gate(State(gate): State<Arc<Gate>>, mut request: Request, next: Next) -> Response {
    if request.uri().path().ends_with("agent-card.json") {
        return next.run(request).await;
    }
    let caller = match gate.authenticator.authenticate(request.headers()) {
        Ok(caller) => caller,
        Err(e) => return refuse(&e),
    };
    if !gate.rate.allow(&caller, Instant::now()) {
        return refuse(&GateError::RateLimited);
    }
    let requested = split_list(
        Some(
            &request
                .headers()
                .get_all("a2a-extensions")
                .iter()
                .filter_map(|v| v.to_str().ok().map(str::to_owned))
                .collect::<Vec<_>>(),
        )
        .filter(|v| !v.is_empty()),
    );
    let active = gate.negotiator.negotiate(&requested);
    let headers = request.headers_mut();
    headers.remove(CALLER_PARAM);
    headers.remove(ACTIVE_EXTENSIONS_PARAM);
    if let Ok(v) = axum::http::HeaderValue::from_str(&caller) {
        headers.insert(CALLER_PARAM, v);
    }
    if let Ok(v) = axum::http::HeaderValue::from_str(&active.join(", ")) {
        headers.insert(ACTIVE_EXTENSIONS_PARAM, v);
    }
    let mut response = next.run(request).await;
    if !active.is_empty()
        && let Ok(v) = axum::http::HeaderValue::from_str(&active.join(", "))
    {
        response.headers_mut().insert("a2a-extensions", v);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::{
        Authenticator, ExtensionNegotiator, GateError, PARTICIPANT_HEADER, RateLimiter,
        StreamBudget, constant_time_eq,
    };
    use crate::host::Secret;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn negotiation_keeps_only_advertised_extensions_in_request_order() {
        let n = ExtensionNegotiator::new([
            "https://x/task/v1".to_owned(),
            "https://x/plan/v1".to_owned(),
        ]);
        let active = n.negotiate(&[
            "https://x/plan/v1".into(),
            "https://evil/v1".into(),
            "https://x/task/v1".into(),
            "https://x/plan/v1".into(),
        ]);
        assert_eq!(active, ["https://x/plan/v1", "https://x/task/v1"]);
    }

    #[test]
    fn credentials_are_compared_in_constant_time_and_the_participant_header_names_the_caller() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        let auth = Authenticator::Bearer(Secret::new("s3cret".to_owned()));
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(
            auth.authenticate(&headers),
            Err(GateError::Unauthenticated),
            "no credential"
        );
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer nope".parse().unwrap(),
        );
        assert_eq!(auth.authenticate(&headers), Err(GateError::Unauthenticated));
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer s3cret".parse().unwrap(),
        );
        assert_eq!(auth.authenticate(&headers).as_deref(), Ok("operator"));
        headers.insert(PARTICIPANT_HEADER, "alice".parse().unwrap());
        assert_eq!(auth.authenticate(&headers).as_deref(), Ok("alice"));
        let key = Authenticator::ApiKey {
            header: "x-api-key".into(),
            key: Secret::new("k".to_owned()),
        };
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-api-key", "k".parse().unwrap());
        assert_eq!(key.authenticate(&headers).as_deref(), Ok("operator"));
        assert_eq!(
            Authenticator::None
                .authenticate(&axum::http::HeaderMap::new())
                .as_deref(),
            Ok("operator")
        );
        assert!(!format!("{auth:?}").contains("s3cret"));
    }

    #[test]
    fn the_token_bucket_refills_per_minute_and_the_stream_budget_releases_on_drop() {
        let limiter = RateLimiter::new(2);
        let t0 = Instant::now();
        assert!(limiter.allow("a", t0) && limiter.allow("a", t0));
        assert!(
            !limiter.allow("a", t0),
            "the third request in the same instant is refused"
        );
        assert!(limiter.allow("b", t0), "another caller has its own bucket");
        assert!(
            limiter.allow("a", t0 + Duration::from_secs(30)),
            "half a minute refills one token"
        );
        let budget = Arc::new(StreamBudget::new(2));
        let s1 = budget.acquire("a").unwrap();
        let _s2 = budget.acquire("a").unwrap();
        assert!(budget.acquire("a").is_none());
        assert!(budget.acquire("b").is_some());
        drop(s1);
        assert_eq!(budget.held("a"), 1);
        assert!(budget.acquire("a").is_some());
    }
}

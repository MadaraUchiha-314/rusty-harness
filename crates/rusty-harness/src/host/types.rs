//! The value types the host capabilities exchange. Deliberately small: each later
//! milestone extends what it needs (streams of audio, MCP channels, process I/O).

use std::collections::BTreeMap;
use std::path::PathBuf;

use bytes::Bytes;

use crate::runtime::HarnessStream;

/// A failure inside a host capability, phrased for the caller, never with a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    /// An I/O failure (file, process, socket).
    #[error("io: {0}")]
    Io(String),
    /// An HTTP-level failure (connection, TLS, a status the caller treats as failure).
    #[error("http: {message}")]
    Http {
        /// The status code when a response arrived.
        status: Option<u16>,
        /// What went wrong.
        message: String,
    },
    /// The operation did not finish in time.
    #[error("timeout")]
    Timeout,
    /// The host refused the operation (an origin off the allowlist, a path outside the
    /// permitted roots, a command the operator did not allow).
    #[error("denied: {0}")]
    Denied(String),
}

/// An outbound HTTP request.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// The method.
    pub method: http::Method,
    /// The absolute URL.
    pub url: url::Url,
    /// The headers; a `Secret` value is written here by the caller for one request and
    /// never kept.
    pub headers: http::HeaderMap,
    /// The body, possibly empty.
    pub body: Bytes,
}

impl HttpRequest {
    /// A request with no headers and no body.
    #[must_use]
    pub fn new(method: http::Method, url: url::Url) -> Self {
        Self {
            method,
            url,
            headers: http::HeaderMap::new(),
            body: Bytes::new(),
        }
    }
}

/// A complete HTTP response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// The status.
    pub status: http::StatusCode,
    /// The headers.
    pub headers: http::HeaderMap,
    /// The whole body.
    pub body: Bytes,
}

/// A streaming HTTP response: the status and headers first, then the body as it arrives.
pub struct HttpStreamingResponse {
    /// The status.
    pub status: http::StatusCode,
    /// The headers.
    pub headers: http::HeaderMap,
    /// The body chunks.
    pub body: ByteStream,
}

impl core::fmt::Debug for HttpStreamingResponse {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HttpStreamingResponse")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// A stream of bytes.
pub type ByteStream = HarnessStream<'static, Result<Bytes, HostError>>;

/// One WebSocket frame the harness cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WsMessage {
    /// A text frame.
    Text(String),
    /// A binary frame.
    Binary(Bytes),
    /// The peer closed.
    Close,
}

/// An audio encoding the speech providers and the surfaces agree on (R28).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AudioFormat {
    /// 16-bit little-endian PCM.
    Pcm16 {
        /// Samples per second.
        sample_rate: u32,
        /// Interleaved channels.
        channels: u8,
    },
    /// Opus frames.
    Opus,
}

/// A stream of encoded audio chunks.
pub type AudioStream = HarnessStream<'static, Result<Bytes, HostError>>;

/// A process to spawn: a program and its arguments, never a shell line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// The executable.
    pub program: PathBuf,
    /// Its arguments, substituted element-wise.
    pub args: Vec<String>,
    /// The environment the child sees: scrubbed, only what is listed here.
    pub env: BTreeMap<String, String>,
    /// The working directory, if not the harness's own.
    pub cwd: Option<PathBuf>,
}

/// How to reach an MCP server (R8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum McpTarget {
    /// A child process speaking MCP over stdio.
    Stdio(Command),
    /// A server speaking MCP's streamable HTTP transport.
    StreamableHttp {
        /// The endpoint.
        url: url::Url,
    },
}

/// A log line, as the `wasm32` host receives it when it supplies the sink (R24.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    /// `trace`, `debug`, `info`, `warn` or `error`.
    pub level: &'static str,
    /// The module path that logged.
    pub target: String,
    /// The message.
    pub message: String,
    /// Structured fields, already redacted.
    pub fields: Vec<(String, String)>,
}

/// A finished span, as the `wasm32` host receives it (R24.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanRecord {
    /// The span's name (a hook point).
    pub name: String,
    /// Duration in microseconds.
    pub duration_us: u64,
    /// Attributes, already redacted.
    pub attributes: Vec<(String, String)>,
}

//! Host capabilities: the only door through which the core touches the platform (R4.7,
//! R29).
//!
//! Natively [`Host::native`] supplies the defaults; on `wasm32` the embedding page hands
//! each capability in through [`HostBuilder`]; an embedding application may replace any
//! one of them. A capability that is missing makes the entity that needs it fail at
//! construction with [`HarnessError::Unsupported`], never at first use (R29.3).

use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Utc};

use crate::error::{Capability, HarnessError};
use crate::harness_async_trait;

mod secret;
pub mod testing;
mod types;

#[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
mod native;

#[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
pub use native::{
    NativeSecrets, OsRng, ReqwestHttp, SystemClock, TokioFs, TokioProcess, scrubbed_env,
};

pub use secret::{InvalidSecretRef, Secret, SecretError, SecretRef};
pub use types::{
    AudioFormat, AudioStream, ByteStream, Command, HostError, HttpRequest, HttpResponse,
    HttpStreamingResponse, LogRecord, McpTarget, SpanRecord, WsMessage,
};

harness_async_trait! {
    /// Outbound HTTP, used by every provider client, the A2A client and remote hooks.
    pub trait HttpClient: Send + Sync {
        /// Sends a request and reads the whole response.
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HostError>;
        /// Sends a request and streams the response body.
        async fn send_streaming(&self, request: HttpRequest) -> Result<HttpStreamingResponse, HostError>;
    }
}

harness_async_trait! {
    /// An open WebSocket.
    pub trait WebSocket: Send {
        /// Sends one frame.
        async fn send(&mut self, message: WsMessage) -> Result<(), HostError>;
        /// Receives the next frame; `None` once closed.
        async fn next(&mut self) -> Option<Result<WsMessage, HostError>>;
        /// Closes the connection.
        async fn close(&mut self) -> Result<(), HostError>;
    }
}

harness_async_trait! {
    /// Opens `WebSocket`s (voice, streaming transcription, MCP upgrades).
    pub trait WebSocketClient: Send + Sync {
        /// Connects to `url` with `headers`.
        async fn connect(&self, url: url::Url, headers: http::HeaderMap) -> Result<Box<dyn WebSocket>, HostError>;
    }
}

/// The current time.
pub trait Clock: Send + Sync {
    /// Now, in UTC.
    fn now(&self) -> DateTime<Utc>;
}

/// A source of randomness.
pub trait Rng: Send + Sync {
    /// Fills `buf` with random bytes.
    fn fill(&self, buf: &mut [u8]);
}

harness_async_trait! {
    /// Turns a [`SecretRef`] into a [`Secret`].
    pub trait SecretResolver: Send + Sync {
        /// Resolves `reference`, or says by name why it could not.
        async fn resolve(&self, reference: &SecretRef) -> Result<Secret<String>, SecretError>;
    }
}

harness_async_trait! {
    /// File access, for plugins, skills, the file store and the configuration file.
    pub trait FileSystem: Send + Sync {
        /// Reads a whole file.
        async fn read(&self, path: &Path) -> Result<Bytes, HostError>;
        /// Writes a whole file atomically (temp file + rename).
        async fn write_atomic(&self, path: &Path, contents: Bytes) -> Result<(), HostError>;
        /// The entries directly under `dir`, as full paths, sorted.
        async fn list(&self, dir: &Path) -> Result<Vec<std::path::PathBuf>, HostError>;
    }
}

harness_async_trait! {
    /// A running child process's standard streams.
    pub trait ChildIo: Send {
        /// Writes to the child's stdin.
        async fn write(&mut self, bytes: Bytes) -> Result<(), HostError>;
        /// Reads the next chunk of the child's stdout; `None` at EOF.
        async fn read(&mut self) -> Option<Result<Bytes, HostError>>;
        /// Kills the child.
        async fn kill(&mut self) -> Result<(), HostError>;
    }
}

harness_async_trait! {
    /// Spawns processes (stdio MCP servers, the embedded Temporal dev server).
    pub trait ProcessSpawner: Send + Sync {
        /// Spawns `command` with piped standard streams.
        async fn spawn(&self, command: Command) -> Result<Box<dyn ChildIo>, HostError>;
    }
}

harness_async_trait! {
    /// A bidirectional MCP channel, as the host supplies it on `wasm32` (Q14).
    pub trait McpChannel: Send {
        /// Sends one JSON-RPC message.
        async fn send(&mut self, message: Bytes) -> Result<(), HostError>;
        /// Receives the next JSON-RPC message; `None` once closed.
        async fn recv(&mut self) -> Option<Result<Bytes, HostError>>;
    }
}

harness_async_trait! {
    /// Opens MCP channels.
    pub trait McpTransport: Send + Sync {
        /// Connects to `target`.
        async fn connect(&self, target: McpTarget) -> Result<Box<dyn McpChannel>, HostError>;
    }
}

harness_async_trait! {
    /// Audio capture (R28.6).
    pub trait Microphone: Send + Sync {
        /// Starts capturing in `format`.
        async fn capture(&self, format: AudioFormat) -> Result<AudioStream, HostError>;
    }
}

harness_async_trait! {
    /// Audio playback (R28.4).
    pub trait Speaker: Send + Sync {
        /// Plays `audio` in `format` until the stream ends.
        async fn play(&self, format: AudioFormat, audio: AudioStream) -> Result<(), HostError>;
    }
}

/// Where logs and spans go when the host owns them (`wasm32`, R24.5).
pub trait LogSink: Send + Sync {
    /// Emits one log line.
    fn emit(&self, record: LogRecord);
    /// Exports one finished span.
    fn export_span(&self, span: SpanRecord);
}

/// The capabilities the harness runs on. Build one with [`Host::builder`] or take
/// [`Host::native`] (native targets) or [`Host::testing`] (tests).
#[derive(Clone, Default)]
pub struct Host {
    http: Option<Arc<dyn HttpClient>>,
    websocket: Option<Arc<dyn WebSocketClient>>,
    clock: Option<Arc<dyn Clock>>,
    rng: Option<Arc<dyn Rng>>,
    secrets: Option<Arc<dyn SecretResolver>>,
    fs: Option<Arc<dyn FileSystem>>,
    process: Option<Arc<dyn ProcessSpawner>>,
    mcp: Option<Arc<dyn McpTransport>>,
    microphone: Option<Arc<dyn Microphone>>,
    speaker: Option<Arc<dyn Speaker>>,
    log_sink: Option<Arc<dyn LogSink>>,
}

impl core::fmt::Debug for Host {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Host")
            .field("supplies", &self.supplied())
            .finish()
    }
}

macro_rules! capability {
    ($field:ident, $getter:ident, $trait:ident, $cap:ident, $doc:literal) => {
        #[doc = $doc]
        ///
        /// # Errors
        ///
        /// [`HarnessError::Unsupported`] when this host does not supply it.
        pub fn $getter(&self) -> Result<Arc<dyn $trait>, HarnessError> {
            self.$field
                .clone()
                .ok_or(HarnessError::Unsupported(Capability::$cap))
        }
    };
}

impl Host {
    /// A host that supplies nothing; add capabilities with the builder.
    #[must_use]
    pub fn builder() -> HostBuilder {
        HostBuilder::default()
    }

    /// The capabilities this host supplies.
    #[must_use]
    pub fn supplied(&self) -> Vec<Capability> {
        let mut out = Vec::new();
        if self.http.is_some() {
            out.push(Capability::Http);
        }
        if self.websocket.is_some() {
            out.push(Capability::WebSocket);
        }
        if self.clock.is_some() {
            out.push(Capability::Clock);
        }
        if self.rng.is_some() {
            out.push(Capability::Rng);
        }
        if self.secrets.is_some() {
            out.push(Capability::SecretResolver);
        }
        if self.fs.is_some() {
            out.push(Capability::FileSystem);
        }
        if self.process.is_some() {
            out.push(Capability::ProcessSpawner);
        }
        if self.mcp.is_some() {
            out.push(Capability::McpTransport);
        }
        if self.microphone.is_some() {
            out.push(Capability::Microphone);
        }
        if self.speaker.is_some() {
            out.push(Capability::Speaker);
        }
        if self.log_sink.is_some() {
            out.push(Capability::LogSink);
        }
        out
    }

    capability!(http, http, HttpClient, Http, "Outbound HTTP.");
    capability!(
        websocket,
        websocket,
        WebSocketClient,
        WebSocket,
        "WebSocket connections."
    );
    capability!(clock, clock, Clock, Clock, "The clock.");
    capability!(rng, rng, Rng, Rng, "The random source.");
    capability!(
        secrets,
        secrets,
        SecretResolver,
        SecretResolver,
        "Credential resolution."
    );
    capability!(fs, fs, FileSystem, FileSystem, "File access.");
    capability!(
        process,
        process,
        ProcessSpawner,
        ProcessSpawner,
        "Process spawning."
    );
    capability!(mcp, mcp, McpTransport, McpTransport, "MCP transports.");
    capability!(
        microphone,
        microphone,
        Microphone,
        Microphone,
        "Audio capture."
    );
    capability!(speaker, speaker, Speaker, Speaker, "Audio playback.");
    capability!(
        log_sink,
        log_sink,
        LogSink,
        LogSink,
        "The host's log and span sink."
    );
}

/// Assembles a [`Host`] capability by capability.
#[derive(Default)]
pub struct HostBuilder {
    host: Host,
}

impl core::fmt::Debug for HostBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HostBuilder")
            .field("supplies", &self.host.supplied())
            .finish()
    }
}

macro_rules! setter {
    ($field:ident, $trait:ident, $doc:literal) => {
        #[doc = $doc]
        #[must_use]
        pub fn $field(mut self, value: Arc<dyn $trait>) -> Self {
            self.host.$field = Some(value);
            self
        }
    };
}

impl HostBuilder {
    setter!(http, HttpClient, "Supplies outbound HTTP.");
    setter!(
        websocket,
        WebSocketClient,
        "Supplies WebSocket connections."
    );
    setter!(clock, Clock, "Supplies the clock.");
    setter!(rng, Rng, "Supplies the random source.");
    setter!(secrets, SecretResolver, "Supplies credential resolution.");
    setter!(fs, FileSystem, "Supplies file access.");
    setter!(process, ProcessSpawner, "Supplies process spawning.");
    setter!(mcp, McpTransport, "Supplies MCP transports.");
    setter!(microphone, Microphone, "Supplies audio capture.");
    setter!(speaker, Speaker, "Supplies audio playback.");
    setter!(log_sink, LogSink, "Supplies the log and span sink.");

    /// The assembled host.
    #[must_use]
    pub fn build(self) -> Host {
        self.host
    }
}

#[cfg(test)]
mod tests {
    use super::{Host, HostBuilder};
    use crate::error::{Capability, HarnessError};
    use std::sync::Arc;

    #[test]
    fn a_missing_capability_is_a_typed_error_at_lookup() {
        let host = HostBuilder::default().build();
        assert_eq!(host.supplied(), Vec::new());
        assert!(matches!(
            host.http(),
            Err(HarnessError::Unsupported(Capability::Http))
        ));
        assert!(matches!(
            host.microphone(),
            Err(HarnessError::Unsupported(Capability::Microphone))
        ));
    }

    #[test]
    fn the_builder_supplies_what_it_was_given_and_nothing_else() {
        let host = Host::builder()
            .clock(Arc::new(super::testing::FixedClock::default()))
            .rng(Arc::new(super::testing::SeededRng::new(7)))
            .build();
        assert_eq!(host.supplied(), [Capability::Clock, Capability::Rng]);
        assert!(host.clock().is_ok());
        assert!(host.fs().is_err());
    }
}

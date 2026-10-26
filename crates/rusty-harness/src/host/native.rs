//! The native defaults behind `rt-tokio`: reqwest for HTTP, the system clock, the OS
//! random source, environment/keyring/file secret resolution, tokio's file system and
//! process spawning. `WebSocket`, MCP transport, microphone and speaker arrive with the
//! milestones that need them; until then a native host reports them unsupported.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::{StreamExt, TryStreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{
    ChildIo, Clock, Command, FileSystem, Host, HostError, HttpClient, HttpRequest, HttpResponse,
    HttpStreamingResponse, ProcessSpawner, Rng, Secret, SecretError, SecretRef, SecretResolver,
    WebSocket, WebSocketClient, WsMessage,
};
use crate::harness_async_trait;

/// reqwest with rustls, no proxy discovery, one shared client.
#[derive(Debug, Clone)]
pub struct ReqwestHttp {
    client: reqwest::Client,
}

impl Default for ReqwestHttp {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("reqwest builds with rustls"),
        }
    }
}

fn into_reqwest(client: &reqwest::Client, request: HttpRequest) -> reqwest::RequestBuilder {
    client
        .request(request.method, request.url)
        .headers(request.headers)
        .body(request.body)
}

fn http_error(err: reqwest::Error) -> HostError {
    HostError::Http {
        status: err.status().map(|s| s.as_u16()),
        message: err.without_url().to_string(),
    }
}

harness_async_trait! {
    impl HttpClient for ReqwestHttp {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HostError> {
            let response = into_reqwest(&self.client, request).send().await.map_err(http_error)?;
            let status = response.status();
            let headers = response.headers().clone();
            let body = response.bytes().await.map_err(http_error)?;
            Ok(HttpResponse { status, headers, body })
        }

        async fn send_streaming(&self, request: HttpRequest) -> Result<HttpStreamingResponse, HostError> {
            let response = into_reqwest(&self.client, request).send().await.map_err(http_error)?;
            let status = response.status();
            let headers = response.headers().clone();
            let body = response.bytes_stream().map_err(http_error).boxed();
            Ok(HttpStreamingResponse { status, headers, body })
        }
    }
}

/// The system clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// The operating system's random source.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsRng;

impl Rng for OsRng {
    fn fill(&self, buf: &mut [u8]) {
        getrandom::fill(buf).expect("the OS random source is available");
    }
}

/// Resolves `env:` from the environment, `file:` from the file system and `keyring:` from
/// the operator's keyring through `secret-tool lookup service <s> username <u>` (GNOME
/// keyring / Secret Service; no extra library). Values are read, wrapped and never logged.
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeSecrets;

fn unresolvable(reference: &SecretRef, reason: impl Into<String>) -> SecretError {
    SecretError::Unresolvable {
        reference: reference.to_string(),
        reason: reason.into(),
    }
}

harness_async_trait! {
    impl SecretResolver for NativeSecrets {
        async fn resolve(&self, reference: &SecretRef) -> Result<Secret<String>, SecretError> {
            match reference {
                SecretRef::Env(name) => match std::env::var(name) {
                    Ok(value) if !value.is_empty() => Ok(Secret::new(value)),
                    Ok(_) => Err(unresolvable(reference, "the variable is empty")),
                    Err(_) => Err(unresolvable(reference, "the variable is not set")),
                },
                SecretRef::File(path) => {
                    let path = home_relative(path).map_err(|e| unresolvable(reference, e))?;
                    tokio::fs::read_to_string(path)
                        .await
                        .map(|s| Secret::new(s.trim_end().to_owned()))
                        .map_err(|e| unresolvable(reference, e.kind().to_string()))
                }
                SecretRef::Keyring { service, user } => {
                    keyring(reference, &[("service", service.as_str()), ("username", user.as_str())]).await
                }
                SecretRef::KeyringAttributes(attributes) => {
                    let pairs: Vec<(&str, &str)> = attributes.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    keyring(reference, &pairs).await
                }
            }
        }
    }
}

/// `~/…` under `$HOME`; any other path as written.
///
/// # Errors
///
/// A path under `~` while `HOME` is not set: read as written it would be a `~` directory
/// in the working directory, whatever happens to be there.
pub(crate) fn home_relative(path: &Path) -> Result<std::path::PathBuf, String> {
    expand_home(path, std::env::var_os("HOME"))
}

fn expand_home(
    path: &Path,
    home: Option<std::ffi::OsString>,
) -> Result<std::path::PathBuf, String> {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => Ok(std::path::PathBuf::from(home).join(rest)),
        (Ok(_), None) => Err(format!(
            "{} is under ~, and HOME is not set",
            path.display()
        )),
        (Err(_), _) => Ok(path.to_path_buf()),
    }
}

/// `secret-tool lookup <attr> <value> …`, with a scrubbed environment.
async fn keyring(
    reference: &SecretRef,
    attributes: &[(&str, &str)],
) -> Result<Secret<String>, SecretError> {
    let mut command = tokio::process::Command::new("secret-tool");
    command.arg("lookup");
    for (k, v) in attributes {
        command.args([k, v]);
    }
    let output = command
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .envs(std::env::var("DBUS_SESSION_BUS_ADDRESS").map(|v| ("DBUS_SESSION_BUS_ADDRESS", v)))
        .envs(std::env::var("XDG_RUNTIME_DIR").map(|v| ("XDG_RUNTIME_DIR", v)))
        .output()
        .await
        .map_err(|e| {
            unresolvable(
                reference,
                format!("secret-tool could not run: {}", e.kind()),
            )
        })?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(unresolvable(reference, "no such keyring entry"));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|_| unresolvable(reference, "the entry is not UTF-8"))?;
    Ok(Secret::new(value.trim_end().to_owned()))
}

/// tokio's file system.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioFs;

fn io_error(path: &Path, err: &std::io::Error) -> HostError {
    HostError::Io(format!("{}: {}", path.display(), err.kind()))
}

harness_async_trait! {
    impl FileSystem for TokioFs {
        async fn read(&self, path: &Path) -> Result<Bytes, HostError> {
            tokio::fs::read(path).await.map(Bytes::from).map_err(|e| io_error(path, &e))
        }

        async fn write_atomic(&self, path: &Path, contents: Bytes) -> Result<(), HostError> {
            let parent = path.parent().ok_or_else(|| HostError::Io(format!("{}: no parent", path.display())))?;
            tokio::fs::create_dir_all(parent).await.map_err(|e| io_error(parent, &e))?;
            let mut tmp = path.as_os_str().to_owned();
            tmp.push(".tmp");
            let tmp = PathBuf::from(tmp);
            tokio::fs::write(&tmp, &contents).await.map_err(|e| io_error(&tmp, &e))?;
            tokio::fs::rename(&tmp, path).await.map_err(|e| io_error(path, &e))
        }

        async fn list(&self, dir: &Path) -> Result<Vec<PathBuf>, HostError> {
            let mut entries = tokio::fs::read_dir(dir).await.map_err(|e| io_error(dir, &e))?;
            let mut out = Vec::new();
            while let Some(entry) = entries.next_entry().await.map_err(|e| io_error(dir, &e))? {
                out.push(entry.path());
            }
            out.sort();
            Ok(out)
        }
    }
}

/// tokio's process spawning with a scrubbed environment: the child sees only what the
/// [`Command`] lists.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioProcess;

struct TokioChild {
    child: tokio::process::Child,
    stdin: Option<tokio::process::ChildStdin>,
    stdout: Option<tokio::process::ChildStdout>,
}

harness_async_trait! {
    impl ChildIo for TokioChild {
        async fn write(&mut self, bytes: Bytes) -> Result<(), HostError> {
            let stdin = self.stdin.as_mut().ok_or_else(|| HostError::Io("stdin closed".into()))?;
            stdin.write_all(&bytes).await.map_err(|e| HostError::Io(e.kind().to_string()))?;
            stdin.flush().await.map_err(|e| HostError::Io(e.kind().to_string()))
        }

        async fn read(&mut self) -> Option<Result<Bytes, HostError>> {
            let stdout = self.stdout.as_mut()?;
            let mut buf = vec![0u8; 8192];
            match stdout.read(&mut buf).await {
                Ok(0) => None,
                Ok(n) => {
                    buf.truncate(n);
                    Some(Ok(Bytes::from(buf)))
                }
                Err(e) => Some(Err(HostError::Io(e.kind().to_string()))),
            }
        }

        async fn kill(&mut self) -> Result<(), HostError> {
            self.child.kill().await.map_err(|e| HostError::Io(e.kind().to_string()))
        }
    }
}

harness_async_trait! {
    impl ProcessSpawner for TokioProcess {
        async fn spawn(&self, command: Command) -> Result<Box<dyn ChildIo>, HostError> {
            let mut cmd = tokio::process::Command::new(&command.program);
            cmd.args(&command.args).env_clear().envs(command.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            if let Some(cwd) = &command.cwd {
                cmd.current_dir(cwd);
            }
            cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).kill_on_drop(true);
            let mut child = cmd.spawn().map_err(|e| HostError::Io(format!("{}: {}", command.program.display(), e.kind())))?;
            let stdin = child.stdin.take();
            let stdout = child.stdout.take();
            Ok(Box::new(TokioChild { child, stdin, stdout }))
        }
    }
}

impl Host {
    /// The native defaults: reqwest, the system clock, the OS random source, the
    /// environment/keyring/file resolver, tokio's file system and process spawning.
    ///
    /// `WebSocket`s come from `tokio-tungstenite` (voice and transcription sessions,
    /// R28.3). MCP transport, microphone and speaker are supplied by later milestones
    /// and report [`crate::HarnessError::Unsupported`] until then.
    #[must_use]
    pub fn native() -> Host {
        let builder = Host::builder()
            .http(Arc::new(ReqwestHttp::default()))
            .websocket(Arc::new(TungsteniteClient))
            .clock(Arc::new(SystemClock))
            .rng(Arc::new(OsRng))
            .secrets(Arc::new(NativeSecrets))
            .fs(Arc::new(TokioFs))
            .process(Arc::new(TokioProcess));
        #[cfg(feature = "audio")]
        let builder = builder
            .microphone(Arc::new(super::audio::CpalMicrophone))
            .speaker(Arc::new(super::audio::CpalSpeaker));
        builder.build()
    }
}

/// `WebSocket`s over `tokio-tungstenite`, TLS through rustls with the OS roots.
#[derive(Debug, Default, Clone, Copy)]
pub struct TungsteniteClient;

type Tungstenite =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// One open connection.
struct TungsteniteSocket(Tungstenite);

fn ws_error(e: &tokio_tungstenite::tungstenite::Error) -> HostError {
    use tokio_tungstenite::tungstenite::Error as E;
    match e {
        E::Http(response) => HostError::Http {
            status: Some(response.status().as_u16()),
            message: "websocket handshake refused".into(),
        },
        // The message never echoes a request header (the bearer token rides on one).
        other => HostError::Io(format!("websocket: {other}")),
    }
}

harness_async_trait! {
    impl WebSocketClient for TungsteniteClient {
        async fn connect(&self, url: url::Url, headers: http::HeaderMap) -> Result<Box<dyn WebSocket>, HostError> {
            use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
            let mut request = url
                .as_str()
                .into_client_request()
                .map_err(|e| ws_error(&e))?;
            request.headers_mut().extend(headers);
            let (stream, _) = tokio_tungstenite::connect_async(request)
                .await
                .map_err(|e| ws_error(&e))?;
            Ok(Box::new(TungsteniteSocket(stream)))
        }
    }
}

harness_async_trait! {
    impl WebSocket for TungsteniteSocket {
        async fn send(&mut self, message: WsMessage) -> Result<(), HostError> {
            use futures::SinkExt as _;
            use tokio_tungstenite::tungstenite::Message as M;
            let frame = match message {
                WsMessage::Text(t) => M::Text(t.into()),
                WsMessage::Binary(b) => M::Binary(b),
                WsMessage::Close => M::Close(None),
            };
            self.0.send(frame).await.map_err(|e| ws_error(&e))
        }

        async fn next(&mut self) -> Option<Result<WsMessage, HostError>> {
            use tokio_tungstenite::tungstenite::Message as M;
            loop {
                return match self.0.next().await? {
                    Ok(M::Text(t)) => Some(Ok(WsMessage::Text(t.to_string()))),
                    Ok(M::Binary(b)) => Some(Ok(WsMessage::Binary(b))),
                    Ok(M::Close(_)) => Some(Ok(WsMessage::Close)),
                    // Pings are answered by tungstenite itself.
                    Ok(M::Ping(_) | M::Pong(_) | M::Frame(_)) => continue,
                    Err(e) => Some(Err(ws_error(&e))),
                };
            }
        }

        async fn close(&mut self) -> Result<(), HostError> {
            self.0.close(None).await.map_err(|e| ws_error(&e))
        }
    }
}

/// Scrubbed environment helper for MCP servers and the dev server (least privilege).
#[must_use]
pub fn scrubbed_env(extra: impl IntoIterator<Item = (String, String)>) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if let Ok(path) = std::env::var("PATH") {
        env.insert("PATH".to_owned(), path);
    }
    env.extend(extra);
    env
}

#[cfg(test)]
mod tests {
    use super::{NativeSecrets, TokioFs, TokioProcess, expand_home, home_relative, scrubbed_env};
    #[cfg(not(feature = "audio"))]
    use crate::error::Capability;
    use crate::host::{Command, FileSystem, Host, ProcessSpawner, SecretRef, SecretResolver};
    use bytes::Bytes;

    #[test]
    fn a_file_reference_under_tilde_is_under_home() {
        let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME is set"));
        assert_eq!(
            home_relative(std::path::Path::new("~/.config/rusty-harness/key")),
            Ok(home.join(".config/rusty-harness/key"))
        );
        assert_eq!(
            home_relative(std::path::Path::new("/run/secrets/key")),
            Ok(std::path::PathBuf::from("/run/secrets/key"))
        );
    }

    /// Without HOME (a bare systemd unit, a container) a path under `~` is an error, not a
    /// `~` directory in the working directory that anything could have planted.
    #[test]
    fn a_path_under_tilde_without_home_is_refused() {
        let refused = expand_home(std::path::Path::new("~/.config/rusty-harness/key"), None);
        assert!(refused.is_err_and(|e| e.contains("HOME is not set")));
        assert_eq!(
            expand_home(std::path::Path::new("/run/secrets/key"), None),
            Ok(std::path::PathBuf::from("/run/secrets/key"))
        );
    }

    #[tokio::test]
    async fn env_and_file_references_resolve_and_missing_ones_fail_by_name() {
        // SAFETY: tests run single-threaded for this case and the variable is unique.
        unsafe { std::env::set_var("RH_TEST_SECRET", "sk-canary-1") };
        let got = NativeSecrets
            .resolve(&SecretRef::Env("RH_TEST_SECRET".into()))
            .await
            .expect("set");
        assert_eq!(got.expose(), "sk-canary-1");
        let err = NativeSecrets
            .resolve(&SecretRef::Env("RH_TEST_MISSING".into()))
            .await
            .expect_err("unset");
        assert!(
            err.to_string().contains("env:RH_TEST_MISSING") && !err.to_string().contains("canary")
        );
        let dir = std::env::temp_dir().join(format!("rh-secret-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp");
        let file = dir.join("token");
        std::fs::write(&file, "sk-canary-2\n").expect("write");
        let got = NativeSecrets
            .resolve(&SecretRef::File(file.clone()))
            .await
            .expect("file");
        assert_eq!(got.expose(), "sk-canary-2");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_native_fs_writes_atomically_and_lists_sorted() {
        let dir = std::env::temp_dir().join(format!("rh-fs-{}", std::process::id()));
        let fs = TokioFs;
        fs.write_atomic(&dir.join("b.txt"), Bytes::from_static(b"b"))
            .await
            .expect("write b");
        fs.write_atomic(&dir.join("a.txt"), Bytes::from_static(b"a"))
            .await
            .expect("write a");
        assert_eq!(
            fs.read(&dir.join("a.txt")).await.expect("read"),
            Bytes::from_static(b"a")
        );
        let names: Vec<_> = fs
            .list(&dir)
            .await
            .expect("list")
            .iter()
            .map(|p| p.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["a.txt", "b.txt"]);
        assert!(!dir.join("a.txt.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_spawned_child_sees_only_the_listed_environment() {
        let child = TokioProcess
            .spawn(Command {
                program: "/usr/bin/env".into(),
                args: vec![],
                env: scrubbed_env([("RH_ONLY".to_owned(), "1".to_owned())]),
                cwd: None,
            })
            .await;
        let Ok(mut child) = child else { return }; // no /usr/bin/env on this machine: nothing to assert
        let mut out = Vec::new();
        while let Some(Ok(chunk)) = child.read().await {
            out.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("RH_ONLY=1"));
        assert!(
            !text.contains("HOME="),
            "the child must not inherit the parent's environment"
        );
    }

    #[test]
    fn the_native_host_supplies_the_m1_set() {
        let host = Host::native();
        assert!(host.http().is_ok() && host.clock().is_ok() && host.rng().is_ok());
        assert!(host.secrets().is_ok() && host.fs().is_ok() && host.process().is_ok());
        assert!(host.websocket().is_ok(), "M6 supplies WebSockets");
        #[cfg(not(feature = "audio"))]
        assert!(matches!(
            host.microphone(),
            Err(crate::HarnessError::Unsupported(Capability::Microphone))
        ));
        #[cfg(feature = "audio")]
        assert!(host.microphone().is_ok() && host.speaker().is_ok());
    }
}

//! Durable modes `remote` and `embedded` on the official Temporal Rust SDK (R22.1); native
//! only.
//!
//! [`TemporalConnection`] is the client: `remote` connects to a Temporal service (Temporal
//! Cloud included) from `[durable.temporal]`, resolving the address, namespace and API key
//! references inside the constructor so no secret leaves it (boundary 6, R22.8), or
//! through the SDK's own environment loader when `from_env` is set; `embedded` starts the
//! SDK's ephemeral dev server (the `temporal` CLI, downloaded once into a cache
//! directory) and is a development mode (Q3). [`TemporalWorker`] runs a worker in this
//! process. Every client carries the [`codec::SecretGuard`] data converter, which refuses
//! a payload holding a resolved secret.

use std::path::PathBuf;
use std::time::Duration;

use temporalio_client::{Client, ClientOptions, Connection, ConnectionOptions};
use temporalio_sdk::{Runtime, Worker, WorkerOptions};

use super::DurableError;
use crate::config::{DurableConfig, DurableMode, TemporalConfig};
use crate::host::{SecretRef, SecretResolver};

pub mod codec;
pub mod execution;
pub mod ping;
pub mod workflow;

pub use codec::SecretGuard;
pub use execution::TemporalExecution;
pub use ping::{PingActivities, PingWorkflow, ping_worker_options};
pub use workflow::{Progress, TaskActivities, TaskWorkflow, TaskWorkflowInput};

/// The identity every connection announces.
const IDENTITY: &str = concat!("rusty-harness/", env!("CARGO_PKG_VERSION"));

/// A connected Temporal client, with the dev server it owns in `embedded` mode.
pub struct TemporalConnection {
    client: Client,
    namespace: String,
    task_queue: String,
    mode: DurableMode,
    guard: SecretGuard,
    #[cfg(feature = "durable-embedded")]
    embedded:
        Option<temporalio_sdk::testing::WorkflowEnvironment<temporalio_sdk::testing::LocalServer>>,
}

impl core::fmt::Debug for TemporalConnection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TemporalConnection")
            .field("mode", &self.mode)
            .field("namespace", &self.namespace)
            .field("task_queue", &self.task_queue)
            .finish_non_exhaustive()
    }
}

/// The connection and client options `remote` uses, from the configuration: every
/// reference resolved here and registered on `guard`, so a payload can never carry it.
///
/// # Errors
///
/// [`DurableError::Failed`] for a missing address or a reference that does not resolve
/// (named by reference, never by value), or an environment profile that does not load.
pub async fn remote_options(
    config: &TemporalConfig,
    secrets: &dyn SecretResolver,
    guard: &SecretGuard,
) -> Result<(ConnectionOptions, ClientOptions), DurableError> {
    if config.from_env {
        let (connection, client) = ClientOptions::load_from_config(
            temporalio_client::envconfig::LoadClientConfigProfileOptions::default(),
        )
        .map_err(|e| DurableError::Failed(format!("temporal environment profile: {e}")))?;
        if let Some(key) = &connection.api_key {
            guard.protect(&crate::host::Secret::new(key.clone()));
        }
        let client = ClientOptions::new(client.namespace)
            .data_converter(guard.data_converter())
            .build();
        return Ok((connection, client));
    }
    let resolve = async |field: &str, reference: &SecretRef| {
        secrets.resolve(reference).await.map_err(|e| {
            DurableError::Failed(format!("durable.temporal.{field} ({reference}): {e}"))
        })
    };
    let Some(address) = &config.address else {
        return Err(DurableError::Failed(
            "durable.temporal.address is required for mode remote (or set from_env)".into(),
        ));
    };
    let address = resolve("address", address).await?;
    let namespace = match &config.namespace {
        Some(reference) => resolve("namespace", reference).await?.expose().clone(),
        None => "default".to_owned(),
    };
    let api_key = match &config.api_key {
        Some(reference) => Some(resolve("api_key", reference).await?),
        None => None,
    };
    let mut target = address.expose().clone();
    if !target.contains("://") {
        target = format!(
            "{}://{target}",
            if api_key.is_some() { "https" } else { "http" }
        );
    }
    let target: url::Url = target.parse().map_err(|e| {
        DurableError::Failed(format!(
            "durable.temporal.address does not parse as a URL: {e}"
        ))
    })?;
    if let Some(key) = &api_key {
        guard.protect(key);
    }
    let connection = ConnectionOptions::new(target)
        .identity(IDENTITY.to_owned())
        .maybe_api_key(api_key.as_ref().map(|k| k.expose().clone()))
        .connect_timeout(Duration::from_secs(10))
        .build();
    let client = ClientOptions::new(namespace)
        .data_converter(guard.data_converter())
        .build();
    Ok((connection, client))
}

impl TemporalConnection {
    /// Connects per `config.mode`: `remote` to the configured service, `embedded` to a dev
    /// server started here. `cache_dir` is where `embedded` keeps the downloaded CLI
    /// (the operating system's temporary directory when `None`).
    ///
    /// # Errors
    ///
    /// [`DurableError::Unreachable`] when the service (or the dev server) cannot be
    /// reached, so the process fails closed at startup; [`DurableError::Failed`] for a
    /// configuration that cannot be used, mode `none` included.
    pub async fn connect(
        config: &DurableConfig,
        secrets: &dyn SecretResolver,
        cache_dir: Option<PathBuf>,
    ) -> Result<Self, DurableError> {
        let guard = SecretGuard::new();
        match config.mode {
            DurableMode::None => Err(DurableError::Failed(
                "durable mode none runs in process; nothing to connect".into(),
            )),
            DurableMode::Remote => {
                let (connection, client_options) =
                    remote_options(&config.temporal, secrets, &guard).await?;
                let namespace = client_options.namespace.clone();
                let connection = Connection::connect(connection)
                    .await
                    .map_err(|e| DurableError::Unreachable(scrub(&guard, &e.to_string())))?;
                let client = Client::new(connection, client_options)
                    .map_err(|e| DurableError::Failed(scrub(&guard, &e.to_string())))?;
                Ok(Self {
                    client,
                    namespace,
                    task_queue: config.temporal.task_queue.clone(),
                    mode: DurableMode::Remote,
                    guard,
                    #[cfg(feature = "durable-embedded")]
                    embedded: None,
                })
            }
            DurableMode::Embedded => Self::embedded(config, cache_dir, guard).await,
        }
    }

    #[cfg(feature = "durable-embedded")]
    async fn embedded(
        config: &DurableConfig,
        cache_dir: Option<PathBuf>,
        guard: SecretGuard,
    ) -> Result<Self, DurableError> {
        use temporalio_sdk::testing::{
            DevServerLogLevel, EphemeralExe, EphemeralExeVersion, LocalWorkflowEnvironmentOptions,
            WorkflowEnvironment,
        };
        // The SDK downloads into the directory but does not create it.
        if let Some(dir) = &cache_dir {
            tokio::fs::create_dir_all(dir).await.map_err(|e| {
                DurableError::Failed(format!("temporal cache directory {}: {e}", dir.display()))
            })?;
        }
        let options = LocalWorkflowEnvironmentOptions::builder()
            .client_options(
                ClientOptions::new("default")
                    .data_converter(guard.data_converter())
                    .build(),
            )
            .server_executable(EphemeralExe::CachedDownload {
                version: EphemeralExeVersion::Default,
                dest_dir: cache_dir.map(|d| d.display().to_string()),
                ttl: None,
            })
            .log_level(DevServerLogLevel::Warn)
            .build();
        let environment = WorkflowEnvironment::start_local(options)
            .await
            .map_err(|e| DurableError::Unreachable(format!("embedded dev server: {e}")))?;
        Ok(Self {
            client: environment.client().clone(),
            namespace: "default".into(),
            task_queue: config.temporal.task_queue.clone(),
            mode: DurableMode::Embedded,
            guard,
            embedded: Some(environment),
        })
    }

    #[cfg(not(feature = "durable-embedded"))]
    async fn embedded(
        _config: &DurableConfig,
        _cache_dir: Option<PathBuf>,
        _guard: SecretGuard,
    ) -> Result<Self, DurableError> {
        Err(DurableError::Failed(
            "durable mode embedded needs the `durable-embedded` feature".into(),
        ))
    }

    /// The namespace-bound client.
    #[must_use]
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The namespace.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The task queue every worker and workflow of this harness uses.
    #[must_use]
    pub fn task_queue(&self) -> &str {
        &self.task_queue
    }

    /// Which mode.
    #[must_use]
    pub fn mode(&self) -> DurableMode {
        self.mode
    }

    /// The guard the client's data converter enforces; register further secrets here.
    #[must_use]
    pub fn guard(&self) -> &SecretGuard {
        &self.guard
    }

    /// Stops the embedded dev server, when there is one.
    pub async fn shutdown(self) {
        #[cfg(feature = "durable-embedded")]
        if let Some(environment) = self.embedded {
            let _ = environment.shutdown().await;
        }
    }
}

/// Never let a connection error echo a resolved value.
fn scrub(guard: &SecretGuard, text: &str) -> String {
    if guard.contains_secret(text.as_bytes()) {
        "[redacted]".to_owned()
    } else {
        text.to_owned()
    }
}

/// A worker running in this process, polling the connection's task queue.
///
/// The SDK's worker future is not `Send`, so it runs on a thread of its own with a
/// current-thread tokio runtime; the client is shared with the rest of the process.
pub struct TemporalWorker {
    shutdown: Box<dyn Fn() + Send + Sync>,
    thread: Option<std::thread::JoinHandle<Result<(), String>>>,
}

impl core::fmt::Debug for TemporalWorker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TemporalWorker").finish_non_exhaustive()
    }
}

impl TemporalWorker {
    /// Starts a worker for `options`.
    ///
    /// # Errors
    ///
    /// [`DurableError::Failed`] when the thread, the SDK runtime or the worker cannot be
    /// created.
    pub fn start(
        connection: &TemporalConnection,
        options: WorkerOptions,
    ) -> Result<Self, DurableError> {
        let client = connection.client.clone();
        let (ready_tx, ready_rx) =
            std::sync::mpsc::channel::<Result<Box<dyn Fn() + Send + Sync>, String>>();
        let thread = std::thread::Builder::new()
            .name("temporal-worker".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("tokio runtime: {e}")));
                        return Err(format!("tokio runtime: {e}"));
                    }
                };
                rt.block_on(async move {
                    let runtime = match Runtime::from_current_tokio(
                        temporalio_sdk::runtime::RuntimeOptions::default(),
                    ) {
                        Ok(r) => r,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("temporal runtime: {e}")));
                            return Err(format!("temporal runtime: {e}"));
                        }
                    };
                    let mut worker = match Worker::new(&runtime, client, options) {
                        Ok(w) => w,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("temporal worker: {e}")));
                            return Err(format!("temporal worker: {e}"));
                        }
                    };
                    let shutdown = worker.shutdown_handle();
                    let _ = ready_tx.send(Ok(Box::new(shutdown)));
                    worker
                        .run()
                        .await
                        .map_err(|e| format!("temporal worker: {e}"))
                })
            })
            .map_err(|e| DurableError::Failed(format!("temporal worker thread: {e}")))?;
        let shutdown = ready_rx
            .recv()
            .map_err(|_| DurableError::Failed("temporal worker thread ended early".into()))?
            .map_err(DurableError::Failed)?;
        Ok(Self {
            shutdown,
            thread: Some(thread),
        })
    }

    /// Asks the worker to stop and waits for it.
    ///
    /// # Errors
    ///
    /// [`DurableError::Failed`] when the worker ended with an error.
    pub async fn stop(mut self) -> Result<(), DurableError> {
        (self.shutdown)();
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || thread.join())
            .await
            .map_err(|e| DurableError::Failed(format!("temporal worker join: {e}")))?
            .map_err(|_| DurableError::Failed("temporal worker thread panicked".into()))?
            .map_err(DurableError::Failed)
    }
}

impl Drop for TemporalWorker {
    fn drop(&mut self) {
        if self.thread.is_some() {
            (self.shutdown)();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SecretGuard, remote_options};
    use crate::app_service::durable::temporal::workflow::TaskWorkflow;

    /// T1: the recorded history of the three-turn scenario replays against the current
    /// `TaskWorkflow` without nondeterminism (the fixture is rewritten by the embedded
    /// scenario under `UPDATE_SNAPSHOTS=1` whenever the workflow changes).
    #[tokio::test]
    async fn the_recorded_history_replays_against_the_current_workflow() {
        use temporalio_client::WorkflowHistory;
        use temporalio_sdk::workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions};
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/temporal/task_workflow_history.json");
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e} (record it with UPDATE_SNAPSHOTS=1)",
                path.display()
            )
        });
        let history = WorkflowHistory::from_json(&bytes).expect("a history");
        let options = WorkflowReplayerOptions::new()
            .register_workflow::<TaskWorkflow>()
            .expect("registers")
            .build();
        let replayer = WorkflowReplayer::new(options).expect("replayer");
        replayer
            .replay_workflow(history)
            .await
            .expect("the history replays against the current workflow");
    }

    use crate::config::TemporalConfig;
    use crate::host::SecretRef;
    use crate::host::testing::MapSecrets;

    fn config(address: Option<&str>, key: Option<&str>) -> TemporalConfig {
        TemporalConfig {
            address: address.map(|a| SecretRef::Env(a.into())),
            namespace: Some(SecretRef::Env("NS".into())),
            api_key: key.map(|k| SecretRef::Env(k.into())),
            from_env: false,
            task_queue: "rusty-harness".into(),
        }
    }

    #[tokio::test]
    async fn remote_options_resolve_references_inside_and_register_the_key_on_the_guard() {
        let secrets = MapSecrets::new([
            (
                SecretRef::Env("ADDR".into()),
                "my-ns.a1b2c.tmprl.cloud:7233".into(),
            ),
            (SecretRef::Env("NS".into()), "my-ns.a1b2c".into()),
            (SecretRef::Env("KEY".into()), "tmprl-canary-key".into()),
        ]);
        let guard = SecretGuard::new();
        let (connection, client) =
            remote_options(&config(Some("ADDR"), Some("KEY")), &secrets, &guard)
                .await
                .expect("options");
        assert_eq!(
            connection.target.as_str(),
            "https://my-ns.a1b2c.tmprl.cloud:7233/"
        );
        assert_eq!(connection.api_key.as_deref(), Some("tmprl-canary-key"));
        assert_eq!(client.namespace, "my-ns.a1b2c");
        assert!(
            guard.contains_secret(b"...tmprl-canary-key..."),
            "the key is guarded"
        );
        assert!(!guard.contains_secret(b"env:KEY"), "the reference is not");

        let err = remote_options(&config(None, None), &secrets, &guard)
            .await
            .expect_err("address required");
        assert!(
            err.to_string().contains("durable.temporal.address"),
            "{err}"
        );
        let err = remote_options(&config(Some("MISSING"), None), &secrets, &guard)
            .await
            .expect_err("unresolvable");
        assert!(err.to_string().contains("env:MISSING"), "{err}");
        assert!(!err.to_string().contains("canary"));
    }
}

//! Python bindings for `rusty_harness`: thin adapters only, no logic.
//!
//! Every async core API is exposed twice — as an awaitable (`await hello()`) driven by
//! the one shared multi-thread tokio runtime that `pyo3-async-runtimes` owns for the
//! module, and as a blocking `*_sync` wrapper for scripts and the REPL (R30.2).
//! Configuration, requests and events cross as plain `dict`s in the shapes the
//! generated `TypedDict`s describe.

use std::sync::Arc;

use futures::StreamExt as _;
use pyo3::exceptions::{PyStopAsyncIteration, PyValueError};
use pyo3::prelude::*;
use pyo3_stub_gen::define_stub_info_gatherer;
use pyo3_stub_gen::derive::{gen_stub_pyclass, gen_stub_pyfunction, gen_stub_pymethods};
use tokio::sync::{Mutex, mpsc, oneshot};

pyo3_stub_gen::create_exception!(
    rusty_harness,
    A2AError,
    pyo3::exceptions::PyException,
    "A rejected call; `code` is the A2A JSON-RPC error code."
);

/// Return a greeting from the harness.
#[gen_stub_pyfunction]
#[gen_stub(override_return_type(
    type_repr = "typing.Awaitable[builtins.str]",
    imports = ("typing", "builtins")
))]
#[pyfunction]
fn hello(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async { Ok(rusty_harness::hello().await) })
}

/// Return a greeting from the harness, blocking until it is ready.
#[gen_stub_pyfunction]
#[pyfunction]
fn hello_sync(py: Python<'_>) -> String {
    py.detach(|| block_on(rusty_harness::hello()))
}

fn block_on<F: Future>(future: F) -> F::Output {
    pyo3_async_runtimes::tokio::get_runtime().block_on(future)
}

fn to_py<T: serde::Serialize>(py: Python<'_>, value: &T) -> PyResult<Py<PyAny>> {
    let text = serde_json::to_string(value).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(py.import("json")?.call_method1("loads", (text,))?.unbind())
}

fn from_py<T: serde::de::DeserializeOwned>(value: &Bound<'_, PyAny>, what: &str) -> PyResult<T> {
    let text: String = value
        .py()
        .import("json")?
        .call_method1("dumps", (value,))?
        .extract()?;
    serde_json::from_str(&text).map_err(|e| PyValueError::new_err(format!("{what}: {e}")))
}

fn layered(file: Option<&str>, code: Option<serde_json::Value>, env: bool) -> PyResult<Config> {
    let vars: Vec<(String, String)> = if env {
        std::env::vars().collect()
    } else {
        Vec::new()
    };
    rusty_harness::config::Config::from_layers_json(file, vars, code)
        .map(|inner| Config { inner })
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

fn a2a_error(e: &a2a::A2AError) -> PyErr {
    let err = A2AError::new_err(e.to_string());
    Python::attach(|py| {
        let _ = err.value(py).setattr("code", e.code);
    });
    err
}

/// The configuration: the same typed `Config` the TOML file expresses (R30.2).
#[gen_stub_pyclass]
#[pyclass(module = "rusty_harness", frozen, skip_from_py_object)]
#[derive(Clone)]
struct Config {
    inner: rusty_harness::config::Config,
}

#[gen_stub_pymethods]
#[pymethods]
impl Config {
    /// The defaults, as `config.default.toml` writes them.
    #[new]
    fn new() -> Self {
        Self {
            inner: rusty_harness::config::Config::default(),
        }
    }

    /// Parses TOML text, unknown fields refused and credentials by reference only. The
    /// `RH__SECTION__KEY` environment variables apply over the file (R30.4) unless `env`
    /// is false.
    #[staticmethod]
    #[pyo3(signature = (text, env = true))]
    fn from_toml(text: &str, env: bool) -> PyResult<Self> {
        layered(Some(text), None, env)
    }

    /// Builds from a `dict` of the `ConfigDict` shape (the TOML file's tables as dicts). The
    /// dict is the code layer: it wins over the `RH__SECTION__KEY` environment variables,
    /// which apply under it unless `env` is false (R30.4).
    #[staticmethod]
    #[pyo3(signature = (value, env = true))]
    fn from_dict(
        #[gen_stub(override_type(type_repr = "ConfigDict", imports = ()))] value: &Bound<'_, PyAny>,
        env: bool,
    ) -> PyResult<Self> {
        layered(None, Some(from_py(value, "config")?), env)
    }

    /// The whole configuration as a `dict`.
    #[gen_stub(override_return_type(type_repr = "ConfigDict", imports = ()))]
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_py(py, &self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "Config(agent={:?}, models={:?})",
            self.inner.agent.name,
            self.inner.models.keys().collect::<Vec<_>>()
        )
    }
}

/// The platform the harness runs on: the native defaults (HTTP, clock, secrets from the
/// environment, keyring or files, the file system, processes).
#[gen_stub_pyclass]
#[pyclass(module = "rusty_harness", frozen)]
struct Host {
    inner: rusty_harness::host::Host,
}

#[gen_stub_pymethods]
#[pymethods]
impl Host {
    /// The native host.
    #[staticmethod]
    fn native() -> Self {
        Self {
            inner: rusty_harness::host::Host::native(),
        }
    }
}

type Item = Result<a2a::StreamResponse, a2a::A2AError>;

/// A turn's events: iterate with `async for` (from the awaitable API) or `for` (from
/// the `*_sync` one).
#[gen_stub_pyclass]
#[pyclass(module = "rusty_harness")]
struct EventStream {
    rx: Arc<Mutex<mpsc::UnboundedReceiver<Item>>>,
}

impl EventStream {
    /// Runs `produce` on the shared runtime from now on, read or not.
    fn spawn<F>(produce: impl FnOnce(mpsc::UnboundedSender<Item>) -> F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        pyo3_async_runtimes::tokio::get_runtime().spawn(produce(tx));
        Self {
            rx: Arc::new(Mutex::new(rx)),
        }
    }
}

#[gen_stub_pymethods]
#[pymethods]
impl EventStream {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    #[gen_stub(override_return_type(
        type_repr = "typing.Awaitable[StreamResponse]",
        imports = ("typing")
    ))]
    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let rx = Arc::clone(&self.rx);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            match rx.lock().await.recv().await {
                Some(Ok(event)) => Python::attach(|py| to_py(py, &event)),
                Some(Err(e)) => Err(a2a_error(&e)),
                None => Err(PyStopAsyncIteration::new_err(())),
            }
        })
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    #[gen_stub(override_return_type(type_repr = "StreamResponse", imports = ()))]
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let rx = Arc::clone(&self.rx);
        match py.detach(|| block_on(async move { rx.lock().await.recv().await })) {
            Some(Ok(event)) => to_py(py, &event).map(Some),
            Some(Err(e)) => Err(a2a_error(&e)),
            None => Ok(None),
        }
    }
}

/// A running A2A server for a harness.
#[gen_stub_pyclass]
#[pyclass(module = "rusty_harness")]
struct Server {
    /// Where it listens, `host:port`.
    #[pyo3(get)]
    address: String,
    stop: Option<oneshot::Sender<()>>,
}

#[gen_stub_pymethods]
#[pymethods]
impl Server {
    /// Stops accepting and lets open streams finish.
    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

async fn start(harness: Arc<rusty_harness::Harness>, bind: Option<String>) -> PyResult<Server> {
    let server = rusty_harness::app_service::a2a_server::serve(&harness, bind.as_deref())
        .await
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let address = server.addr.to_string();
    let (stop, stopped) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let _keep = harness;
        if let Err(e) = server
            .run(async move {
                let _ = stopped.await;
            })
            .await
        {
            tracing::warn!(error = %e, "the A2A server stopped");
        }
    });
    Ok(Server {
        address,
        stop: Some(stop),
    })
}

/// A harness in this process: what the CLI assembles, from the same `Config` (R30).
#[gen_stub_pyclass]
#[pyclass(module = "rusty_harness", frozen)]
struct Harness {
    inner: Arc<rusty_harness::Harness>,
}

async fn build(
    config: rusty_harness::config::Config,
    host: Option<rusty_harness::host::Host>,
) -> PyResult<Harness> {
    let mut builder = rusty_harness::HarnessBuilder::new(config);
    if let Some(host) = host {
        builder = builder.host(host);
    }
    builder
        .build()
        .await
        .map(|inner| Harness {
            inner: Arc::new(inner),
        })
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

#[gen_stub_pymethods]
#[pymethods]
impl Harness {
    /// Builds a harness, blocking until it is ready; `Harness.create` is the awaitable.
    #[new]
    #[pyo3(signature = (config, host = None))]
    fn new(py: Python<'_>, config: &Config, host: Option<&Host>) -> PyResult<Self> {
        let (config, host) = (config.inner.clone(), host.map(|h| h.inner.clone()));
        py.detach(|| block_on(build(config, host)))
    }

    /// Builds a harness; fails closed when a configured credential does not resolve.
    #[staticmethod]
    #[pyo3(signature = (config, host = None))]
    #[gen_stub(override_return_type(type_repr = "typing.Awaitable[Harness]", imports = ("typing")))]
    fn create<'py>(
        py: Python<'py>,
        config: &Config,
        host: Option<&Host>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (config, host) = (config.inner.clone(), host.map(|h| h.inner.clone()));
        pyo3_async_runtimes::tokio::future_into_py(py, build(config, host))
    }

    /// Builds a harness, blocking until it is ready.
    #[staticmethod]
    #[pyo3(signature = (config, host = None))]
    fn create_sync(py: Python<'_>, config: &Config, host: Option<&Host>) -> PyResult<Self> {
        Self::new(py, config, host)
    }

    /// Sends a message; `async for event in harness.send_message(request)`.
    #[gen_stub(override_return_type(
        type_repr = "typing.AsyncIterator[StreamResponse]",
        imports = ("typing")
    ))]
    fn send_message(
        &self,
        #[gen_stub(override_type(type_repr = "SendMessageRequest", imports = ()))] request: &Bound<
            '_,
            PyAny,
        >,
    ) -> PyResult<EventStream> {
        let request: a2a::SendMessageRequest = from_py(request, "request")?;
        let inner = Arc::clone(&self.inner);
        Ok(EventStream::spawn(move |tx| async move {
            let mut events = inner.send_message(request, None);
            while let Some(event) = events.next().await {
                // The turn runs to its end even when nobody reads, as on the server.
                let _ = tx.send(event);
            }
        }))
    }

    /// Sends a message; `for event in harness.send_message_sync(request)`.
    #[gen_stub(override_return_type(
        type_repr = "typing.Iterator[StreamResponse]",
        imports = ("typing")
    ))]
    fn send_message_sync(
        &self,
        #[gen_stub(override_type(type_repr = "SendMessageRequest", imports = ()))] request: &Bound<
            '_,
            PyAny,
        >,
    ) -> PyResult<EventStream> {
        self.send_message(request)
    }

    /// The task's log so far, then its live events until the turn ends.
    #[gen_stub(override_return_type(
        type_repr = "typing.AsyncIterator[StreamResponse]",
        imports = ("typing")
    ))]
    fn subscribe(&self, task_id: String) -> EventStream {
        let inner = Arc::clone(&self.inner);
        EventStream::spawn(move |tx| async move {
            match inner.subscribe(&task_id, None).await {
                Ok(mut events) => {
                    while let Some(event) = events.next().await {
                        let _ = tx.send(event);
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        })
    }

    /// The task's log so far, then its live events, blocking per event.
    #[gen_stub(override_return_type(
        type_repr = "typing.Iterator[StreamResponse]",
        imports = ("typing")
    ))]
    fn subscribe_sync(&self, task_id: String) -> EventStream {
        self.subscribe(task_id)
    }

    /// The task as it stands; raises `A2AError` when there is none.
    #[gen_stub(override_return_type(type_repr = "typing.Awaitable[Task]", imports = ("typing")))]
    fn get_task<'py>(&self, py: Python<'py>, task_id: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let task = inner
                .get_task(&task_id, None)
                .await
                .map_err(|e| a2a_error(&e))?;
            Python::attach(|py| to_py(py, &task))
        })
    }

    /// The task as it stands, blocking.
    #[gen_stub(override_return_type(type_repr = "Task", imports = ()))]
    fn get_task_sync(&self, py: Python<'_>, task_id: String) -> PyResult<Py<PyAny>> {
        let inner = Arc::clone(&self.inner);
        let task = py
            .detach(|| block_on(async move { inner.get_task(&task_id, None).await }))
            .map_err(|e| a2a_error(&e))?;
        to_py(py, &task)
    }

    /// Cancels a task; raises `A2AError` when there is none or it already ended.
    #[gen_stub(override_return_type(type_repr = "typing.Awaitable[Task]", imports = ("typing")))]
    fn cancel_task<'py>(&self, py: Python<'py>, task_id: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let task = inner
                .cancel_task(&task_id, None)
                .await
                .map_err(|e| a2a_error(&e))?;
            Python::attach(|py| to_py(py, &task))
        })
    }

    /// Cancels a task, blocking.
    #[gen_stub(override_return_type(type_repr = "Task", imports = ()))]
    fn cancel_task_sync(&self, py: Python<'_>, task_id: String) -> PyResult<Py<PyAny>> {
        let inner = Arc::clone(&self.inner);
        let task = py
            .detach(|| block_on(async move { inner.cancel_task(&task_id, None).await }))
            .map_err(|e| a2a_error(&e))?;
        to_py(py, &task)
    }

    /// Serves the harness over A2A on `bind` (default `[server] bind`) and returns once
    /// it listens; without `[server.security]` only a loopback address is accepted.
    #[pyo3(signature = (bind = None))]
    #[gen_stub(override_return_type(type_repr = "typing.Awaitable[Server]", imports = ("typing")))]
    fn serve<'py>(&self, py: Python<'py>, bind: Option<String>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, start(inner, bind))
    }

    /// Serves the harness over A2A, blocking until it listens.
    #[pyo3(signature = (bind = None))]
    fn serve_sync(&self, py: Python<'_>, bind: Option<String>) -> PyResult<Server> {
        let inner = Arc::clone(&self.inner);
        py.detach(|| block_on(start(inner, bind)))
    }

    /// The configuration the harness was built from.
    #[getter]
    fn config(&self) -> Config {
        Config {
            inner: self.inner.config().clone(),
        }
    }
}

/// A tiny agent harness that's blazing fast.
#[pymodule]
#[pyo3(name = "rusty_harness")]
fn rusty_harness_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    m.add_function(wrap_pyfunction!(hello_sync, m)?)?;
    m.add_class::<Config>()?;
    m.add_class::<Host>()?;
    m.add_class::<Harness>()?;
    m.add_class::<EventStream>()?;
    m.add_class::<Server>()?;
    m.add("A2AError", m.py().get_type::<A2AError>())?;
    Ok(())
}

define_stub_info_gatherer!(stub_info);

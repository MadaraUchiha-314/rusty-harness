//! Python bindings for `rusty_harness`: thin adapters only, no logic.
//!
//! Every async core API is exposed twice — as an awaitable (`await hello()`) driven by
//! the one shared multi-thread tokio runtime that `pyo3-async-runtimes` owns for the
//! module, and as a blocking `*_sync` wrapper for scripts and the REPL.

use pyo3::prelude::*;
use pyo3_stub_gen::define_stub_info_gatherer;
use pyo3_stub_gen::derive::gen_stub_pyfunction;

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
    py.detach(|| pyo3_async_runtimes::tokio::get_runtime().block_on(rusty_harness::hello()))
}

/// A tiny agent harness that's blazing fast.
#[pymodule]
#[pyo3(name = "rusty_harness")]
fn rusty_harness_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    m.add_function(wrap_pyfunction!(hello_sync, m)?)?;
    Ok(())
}

define_stub_info_gatherer!(stub_info);

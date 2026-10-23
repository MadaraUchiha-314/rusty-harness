//! Regenerates `rusty_harness.pyi` from the `#[gen_stub_*]` annotations.

fn main() -> pyo3_stub_gen::Result<()> {
    rusty_harness_py::stub_info()?.generate()?;
    Ok(())
}

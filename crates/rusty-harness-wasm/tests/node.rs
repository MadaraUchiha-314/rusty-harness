//! Runs in Node via `wasm-pack test --node`; `browser.rs` covers the browser.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test(async)]
async fn hello_resolves() {
    assert_eq!(
        rusty_harness_wasm::hello().await,
        "Hello from rusty-harness!"
    );
}

/// Feature: Host capabilities on wasm32
/// Requirement: docs/specs/issue-3/requirements.md#R29
///
/// Scenario: An unavailable capability fails at construction on wasm32
///     Given a host the page built with no file system
///     When the harness asks the host for file access
///     Then it gets a typed Unsupported(FileSystem) error, before any call could use it
#[wasm_bindgen_test]
fn an_unavailable_capability_fails_at_construction_on_wasm32() {
    use rusty_harness::host::Host;
    use rusty_harness::{Capability, HarnessError};
    let host = Host::builder().build();
    assert!(matches!(host.fs(), Err(HarnessError::Unsupported(Capability::FileSystem))));
    assert!(host.supplied().is_empty());
}

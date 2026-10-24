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

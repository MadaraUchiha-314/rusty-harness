//! Runs in a headless browser via `wasm-pack test --headless --chrome`; `hello.rs` covers Node.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test(async)]
async fn hello_resolves_in_the_browser() {
    assert_eq!(
        rusty_harness_wasm::hello().await,
        "Hello from rusty-harness!"
    );
}

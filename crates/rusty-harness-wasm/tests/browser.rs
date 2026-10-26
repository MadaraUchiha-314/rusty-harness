//! Runs in a headless browser via `wasm-pack test --headless --chrome`; `hello.rs` covers Node.
#![cfg(target_arch = "wasm32")]

mod scenario;

use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test(async)]
async fn hello_resolves_in_the_browser() {
    assert_eq!(
        rusty_harness_wasm::hello().await,
        "Hello from rusty-harness!"
    );
}

#[wasm_bindgen_test(async)]
async fn the_wasm_build_runs_a_task_against_a_mocked_host() {
    scenario::the_wasm_build_runs_a_task_against_a_mocked_host().await;
}

#[wasm_bindgen_test(async)]
async fn a_fetch_outside_allow_origins_is_refused() {
    scenario::a_fetch_outside_allow_origins_is_refused().await;
}

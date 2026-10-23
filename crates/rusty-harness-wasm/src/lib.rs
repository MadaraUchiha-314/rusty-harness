//! WebAssembly bindings for `rusty_harness`: thin adapters only, no logic.
//!
//! Every async core API is exported as a `Promise`-returning function.

use wasm_bindgen::prelude::*;

/// Return a greeting from the harness.
#[wasm_bindgen]
pub async fn hello() -> String {
    rusty_harness::hello().await
}

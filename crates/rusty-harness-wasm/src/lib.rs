//! WebAssembly bindings for `rusty_harness`: thin adapters only, no logic.
//!
//! Every async core API is exported as a `Promise`-returning function; streams are
//! `AsyncIterable`s (R29.4, design.md § Packaging and SDK parity).

use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
mod sdk;

#[cfg(target_arch = "wasm32")]
pub use sdk::Harness;

/// Return a greeting from the harness.
#[wasm_bindgen]
pub async fn hello() -> String {
    rusty_harness::hello().await
}

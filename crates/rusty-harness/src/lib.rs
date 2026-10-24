//! `rusty_harness` — a tiny agent harness that's blazing fast.
//!
//! The crate is async by default and runtime-agnostic: public APIs are `async fn`s that
//! return plain futures, so you drive them with whichever executor you already use.
//! Anything that needs a runtime (spawning, timers) goes through the [`Runtime`] trait,
//! implemented behind the `rt-tokio` and `rt-wasm` features.
//!
//! ```
//! # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
//! let greeting = rusty_harness::hello().await;
//! assert_eq!(greeting, "Hello from rusty-harness!");
//! # });
//! ```
#![deny(missing_docs)]

pub mod app_service;
pub mod assets;
pub mod config;
pub mod error;
pub mod harness;
pub mod host;
pub mod interaction;
pub mod registry;
pub mod runtime;
pub mod self_improvement;

pub use error::{Capability, HarnessError};
pub use runtime::{HarnessFuture, HarnessStream, MaybeSend, MaybeSync, Runtime};

/// Returns a greeting from the harness.
///
/// ```
/// # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
/// assert_eq!(rusty_harness::hello().await, "Hello from rusty-harness!");
/// # });
/// ```
#[allow(
    clippy::unused_async,
    reason = "the public API is async by design; real APIs will await"
)]
pub async fn hello() -> String {
    "Hello from rusty-harness!".to_owned()
}

#[cfg(all(test, feature = "rt-tokio"))]
mod tests {
    use super::hello;

    #[tokio::test]
    async fn hello_greets() {
        assert_eq!(hello().await, "Hello from rusty-harness!");
    }
}

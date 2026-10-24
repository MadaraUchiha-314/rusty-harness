//! Durable execution: the `DurableExecution` trait, the in-process `none` mode and the Temporal modes (R22).

pub mod none;
#[cfg(all(feature = "durable-temporal", not(target_arch = "wasm32")))]
pub mod temporal;

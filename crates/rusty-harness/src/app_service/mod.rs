//! Layer 2 of the architecture: the app service layer.

#[cfg(feature = "server")]
pub mod a2a_server;
pub mod channels_api;
pub mod durable;
pub mod heartbeat;
pub mod inbox;
pub mod o11y;

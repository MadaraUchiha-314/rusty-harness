//! The error taxonomy: one `HarnessError`, typed at every boundary, mapped once to A2A
//! errors (design.md § Error handling).
//!
//! Each module adds its own variant as it lands; the mapping to the protocol's error
//! codes lives here so the server never invents one.

/// A capability the current target or host does not supply (R29.3).
///
/// On `wasm32` only what the host handed in exists; natively the defaults cover
/// everything. The entity that needs a missing capability fails at construction, never
/// at first use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Capability {
    /// HTTP requests.
    Http,
    /// WebSocket connections.
    WebSocket,
    /// A clock.
    Clock,
    /// A random source.
    Rng,
    /// Resolving credential references.
    SecretResolver,
    /// File access.
    FileSystem,
    /// Spawning processes.
    ProcessSpawner,
    /// A transport to an MCP server.
    McpTransport,
    /// Audio capture.
    Microphone,
    /// Audio playback.
    Speaker,
    /// Log and span export.
    LogSink,
    /// Durable execution on Temporal.
    DurableTemporal,
}

/// Every failure the harness reports (design.md § Error handling).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HarnessError {
    /// A protocol error from the A2A layer, passed through unchanged.
    #[error("a2a: {}", .0.message)]
    A2a(#[from] a2a::A2AError),
    /// The target or host lacks a capability (R29.3).
    #[error("unsupported on this target or host: {0:?}")]
    Unsupported(Capability),
    /// A task transition the state machine forbids (R21).
    #[error("illegal task transition from {from:?} to {to}")]
    IllegalTransition {
        /// The task's state before the attempt.
        from: a2a::TaskState,
        /// The transition that was asked for.
        to: &'static str,
    },
    /// A `pre` or `guard` hook said no (R4.2).
    #[error("vetoed by {}: {}", .0.by, .0.reason)]
    Vetoed(crate::harness::hooks::Veto),
    /// A hook failed under a fail-closed policy, or an `in` hook moved the target (R4.6).
    #[error("hook at {point}/{phase:?} failed ({policy:?}): {source}")]
    Hook {
        /// The catalogue point.
        point: crate::harness::hooks::HookPoint,
        /// The stage that failed.
        phase: crate::harness::hooks::Phase,
        /// What the hook reported.
        source: crate::harness::hooks::HookError,
        /// The policy that turned it into a failure.
        policy: crate::harness::hooks::FailurePolicy,
    },
    /// A failure with no more specific variant yet; never the answer to a caller's input.
    #[error("internal: {0}")]
    Internal(String),
}

impl HarnessError {
    /// The protocol error the A2A server answers with (R19.4, R20.6).
    ///
    /// The mapping is the only place a harness failure becomes a protocol code, so the
    /// server cannot leak a different code per call site.
    #[must_use]
    pub fn to_a2a(&self) -> a2a::A2AError {
        match self {
            Self::A2a(err) => err.clone(),
            Self::Unsupported(cap) => {
                a2a::A2AError::unsupported_operation(format!("unsupported: {cap:?}"))
            }
            Self::IllegalTransition { from, to } => a2a::A2AError::unsupported_operation(format!(
                "task cannot move from {from:?} to {to}"
            )),
            Self::Vetoed(veto) => {
                a2a::A2AError::unsupported_operation(format!("vetoed: {}", veto.reason))
            }
            Self::Hook { point, phase, .. } => {
                a2a::A2AError::internal(format!("hook failure at {point}/{phase:?}"))
            }
            Self::Internal(msg) => a2a::A2AError::internal(msg.clone()),
        }
    }
}

/// The crate's result type.
pub type Result<T, E = HarnessError> = core::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::{Capability, HarnessError};

    #[test]
    fn a2a_errors_pass_through_unchanged() {
        let err = HarnessError::from(a2a::A2AError::task_not_found("t1"));
        assert_eq!(err.to_a2a().code, a2a::error_code::TASK_NOT_FOUND);
    }

    #[test]
    fn unsupported_and_illegal_transition_map_to_unsupported_operation() {
        let unsupported = HarnessError::Unsupported(Capability::Microphone);
        assert_eq!(
            unsupported.to_a2a().code,
            a2a::error_code::UNSUPPORTED_OPERATION
        );
        let illegal = HarnessError::IllegalTransition {
            from: a2a::TaskState::Completed,
            to: "complete",
        };
        assert_eq!(
            illegal.to_a2a().code,
            a2a::error_code::UNSUPPORTED_OPERATION
        );
        assert!(illegal.to_string().contains("Completed"));
    }

    #[test]
    fn internal_never_names_a_caller_error_code() {
        let err = HarnessError::Internal("disk".into());
        assert_eq!(err.to_a2a().code, a2a::error_code::INTERNAL_ERROR);
    }
}

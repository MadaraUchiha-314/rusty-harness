//! Durable execution: the `DurableExecution` trait, the in-process `none` mode and the
//! Temporal modes (R22).
//!
//! The workflow is started by the inbox, one per task, and the loop's side effects are
//! its activities (design.md § Durable execution). [`RetryPolicy`] is the one retry
//! schedule every mode applies, orchestrated by the mode itself rather than an engine.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::app_service::inbox::EventRef;
use crate::config::{DurableMode, RetryConfig};
use crate::harness::core_loop::task::{ParticipantId, TaskId};
use crate::harness_async_trait;
use crate::registry::Entity;

pub mod none;
#[cfg(all(feature = "durable-temporal", not(target_arch = "wasm32")))]
pub mod temporal;

/// Why a task was cancelled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum CancelReason {
    /// A participant sent `CancelTask`.
    Participant {
        /// Who.
        by: ParticipantId,
    },
    /// The parent task reached a terminal state (R12.4).
    ParentTerminal {
        /// The parent.
        parent: TaskId,
    },
    /// The operator or a hook.
    Policy {
        /// A string-catalog key.
        reason: String,
    },
}

/// A typed durable-execution error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DurableError {
    /// The engine is not reachable (fail-closed at startup).
    #[error("durable execution unreachable: {0}")]
    Unreachable(String),
    /// No such task.
    #[error("task {0}: not found")]
    NotFound(TaskId),
    /// Something the mode could not do.
    #[error("durable execution: {0}")]
    Failed(String),
}

harness_async_trait! {
    /// Runs the loop durably (R22): a mode starts the workflow, signals it, cancels or
    /// completes it, and resumes every non-terminal task after a restart (R14.4).
    pub trait DurableExecution: Entity {
        /// Starts the task's workflow or signals the running one; idempotent on the task id.
        async fn start_or_signal(&self, task: &TaskId, event: EventRef) -> Result<(), DurableError>;
        /// The stop path ending in `canceled`.
        async fn cancel(&self, task: &TaskId, reason: CancelReason) -> Result<(), DurableError>;
        /// The stop path ending in `completed`, keeping the last delivered output.
        async fn complete(&self, task: &TaskId, by: ParticipantId) -> Result<(), DurableError>;
        /// Resumes `tasks` after a restart.
        async fn resume_all(&self, tasks: Vec<TaskId>) -> Result<(), DurableError>;
        /// Which mode this is.
        fn mode(&self) -> DurableMode;
    }
}

/// One activity attempt, as the `durable.*` hook points name it (R22.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityRef {
    /// The task.
    pub task: TaskId,
    /// The iteration.
    pub iteration: u32,
    /// The attempt within the iteration.
    pub attempt: u32,
    /// Which activity: `build_context`, `call_model`, `execute_tool`, `finish_iteration`.
    pub activity: String,
}

/// Why an attempt failed, as `durable.activity_failure` sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityFailure {
    /// The attempt.
    pub activity: ActivityRef,
    /// The error, rendered.
    pub error: String,
    /// Whether a timeout (a crashed or stalled worker) rather than the activity's own
    /// failure caused it.
    pub timeout: bool,
}

/// What happens next, decided under the schedule and open to `durable.activity_retry`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "decision")]
pub enum RetryDecision {
    /// Try again after the wait, as the next attempt.
    Retry {
        /// The wait, in milliseconds (a deterministic workflow timer).
        after_ms: u64,
    },
    /// Give up: the task fails with the reason.
    Stop {
        /// Why.
        reason: String,
    },
}

/// The input of `durable.activity_retry`: the failure and the schedule's own answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryProposal {
    /// The failure.
    pub failure: ActivityFailure,
    /// What the schedule proposes.
    pub proposed: RetryDecision,
}

/// The `durable.activity_run` operation: wraps one attempt; the output is the activity's
/// result as JSON, so an `in` hook can observe or replace it.
#[derive(Debug)]
pub struct ActivityRun;

impl crate::harness::hooks::Operation for ActivityRun {
    const POINT: crate::harness::hooks::HookPoint =
        crate::harness::hooks::HookPoint::DurableActivityRun;
    type Input = ActivityRef;
    type Output = serde_json::Value;
    fn identity(input: &ActivityRef) -> String {
        format!(
            "{}:{}:{}:{}",
            input.task, input.iteration, input.attempt, input.activity
        )
    }
}

/// The `durable.activity_failure` operation: a failed attempt, for the record.
#[derive(Debug)]
pub struct ActivityFailed;

impl crate::harness::hooks::Operation for ActivityFailed {
    const POINT: crate::harness::hooks::HookPoint =
        crate::harness::hooks::HookPoint::DurableActivityFailure;
    type Input = ActivityFailure;
    type Output = ();
    fn identity(input: &ActivityFailure) -> String {
        ActivityRun::identity(&input.activity)
    }
    fn attributes(input: &ActivityFailure, (): &()) -> Vec<(&'static str, String)> {
        vec![("timeout", input.timeout.to_string())]
    }
}

/// The `durable.activity_retry` operation: the schedule's proposal in, the decision out.
#[derive(Debug)]
pub struct ActivityRetry;

impl crate::harness::hooks::Operation for ActivityRetry {
    const POINT: crate::harness::hooks::HookPoint =
        crate::harness::hooks::HookPoint::DurableActivityRetry;
    type Input = RetryProposal;
    type Output = RetryDecision;
    fn identity(input: &RetryProposal) -> String {
        ActivityRun::identity(&input.failure.activity)
    }
}

/// Per-activity timeouts (R22.5): start-to-close, and the heartbeat bound that detects a
/// crashed worker (R23.4). Milliseconds, so the workflow's input stays deterministic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityTimeouts {
    /// `build_context`, `finish_iteration`, `set_state`, `retry_decision`.
    pub short_ms: u64,
    /// `call_model` start-to-close.
    pub model_ms: u64,
    /// `call_model` heartbeat.
    pub model_heartbeat_ms: u64,
    /// `execute_tool` start-to-close.
    pub tool_ms: u64,
    /// `execute_tool` heartbeat.
    pub tool_heartbeat_ms: u64,
}

impl Default for ActivityTimeouts {
    /// The design's values: 30 s, 10 min / 30 s, 5 min / 15 s.
    fn default() -> Self {
        Self {
            short_ms: 30_000,
            model_ms: 600_000,
            model_heartbeat_ms: 30_000,
            tool_ms: 300_000,
            tool_heartbeat_ms: 15_000,
        }
    }
}

/// The retry schedule (R22.3): exponential backoff from `initial` by `coefficient`,
/// capped at `max_interval`, at most `max_attempts` attempts in total.
#[derive(Clone, Debug, PartialEq)]
pub struct RetryPolicy {
    /// The first wait.
    pub initial: Duration,
    /// Multiplier per attempt.
    pub coefficient: f64,
    /// The longest wait.
    pub max_interval: Duration,
    /// Attempts in total.
    pub max_attempts: u32,
}

impl From<&RetryConfig> for RetryPolicy {
    fn from(c: &RetryConfig) -> Self {
        Self {
            initial: c.initial.into(),
            coefficient: f64::from(c.coefficient),
            max_interval: c.max_interval.into(),
            max_attempts: c.max_attempts.max(1),
        }
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::from(&RetryConfig::default())
    }
}

impl RetryPolicy {
    /// The wait before attempt `next` (from 2); `None` when no attempt is left.
    #[must_use]
    pub fn backoff(&self, next: u32) -> Option<Duration> {
        if next < 2 || next > self.max_attempts {
            return None;
        }
        let factor = self
            .coefficient
            .powi(i32::try_from(next - 2).unwrap_or(i32::MAX));
        let wait = self.initial.mul_f64(factor.max(1.0));
        Some(wait.min(self.max_interval))
    }
}

#[cfg(test)]
mod tests {
    use super::RetryPolicy;
    use std::time::Duration;

    #[test]
    fn the_schedule_doubles_from_the_first_wait_and_stays_finite() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_attempts, 5);
        assert_eq!(p.backoff(1), None, "the first attempt waits for nothing");
        assert_eq!(p.backoff(2), Some(Duration::from_secs(1)));
        assert_eq!(p.backoff(3), Some(Duration::from_secs(2)));
        assert_eq!(p.backoff(5), Some(Duration::from_secs(8)));
        assert_eq!(p.backoff(6), None, "no sixth attempt");
        let capped = RetryPolicy {
            max_interval: Duration::from_secs(3),
            max_attempts: 9,
            ..p
        };
        assert_eq!(capped.backoff(9), Some(Duration::from_secs(3)));
    }
}

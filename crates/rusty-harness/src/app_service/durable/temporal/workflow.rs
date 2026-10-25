//! `TaskWorkflow` and its activities (R22.2, R22.4, R22.6): the loop's four phases as
//! activities, the deterministic body that schedules them, and the signals and query
//! the inbox and the server use. Activities exchange store keys and small summaries;
//! every payload is an id, a count or an outcome, never content.
//!
//! The SDK's macros generate items this crate's lints cannot document or type, hence the
//! module-level allowances.
#![allow(
    missing_docs,
    missing_debug_implementations,
    clippy::missing_errors_doc,
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    clippy::large_futures, // the phases are the agent's futures, run once per activity
    clippy::must_use_candidate // the macro's signatures
)]

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use temporalio_sdk::{
    ActivityExecutionError, ActivityOptions, ApplicationFailure, SyncWorkflowContext,
    WorkflowContext, WorkflowContextView, WorkflowResult,
};

use crate::app_service::durable::CancelReason;
use crate::app_service::inbox::EventRef;
use crate::harness::core_loop::agent::{
    AgentError, Begun, IterationOutcome, LocalAgent, LoopOutcome, ModelPhase,
};
use crate::harness::core_loop::task::{Attempt, ParticipantId, TaskId};

/// What starts a task's workflow: the task and the retry schedule, so the body stays
/// deterministic without reading configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskWorkflowInput {
    pub task: TaskId,
    pub retry: RetrySchedule,
}

/// The retry schedule (R22.3), in whole milliseconds for a deterministic body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrySchedule {
    pub initial_ms: u64,
    pub coefficient: u32,
    pub max_interval_ms: u64,
    pub max_attempts: u32,
}

impl RetrySchedule {
    /// The wait before attempt `next` (from 2); `None` when no attempt is left.
    #[must_use]
    pub fn backoff(&self, next: u32) -> Option<Duration> {
        if next < 2 || next > self.max_attempts {
            return None;
        }
        let factor = u64::from(self.coefficient.max(1)).saturating_pow(next - 2);
        let wait = self
            .initial_ms
            .saturating_mul(factor)
            .min(self.max_interval_ms);
        Some(Duration::from_millis(wait))
    }
}

impl From<&super::super::RetryPolicy> for RetrySchedule {
    fn from(p: &super::super::RetryPolicy) -> Self {
        Self {
            initial_ms: u64::try_from(p.initial.as_millis()).unwrap_or(u64::MAX),
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            coefficient: p.coefficient.max(1.0) as u32,
            max_interval_ms: u64::try_from(p.max_interval.as_millis()).unwrap_or(u64::MAX),
            max_attempts: p.max_attempts.max(1),
        }
    }
}

/// The `progress` query's answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub iteration: u32,
    pub phase: String,
    pub attempt: u32,
    pub inbox_signals: u64,
}

/// A phase's address: the task and the iteration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IterationStep {
    pub task: TaskId,
    pub iteration: u32,
}

/// A phase's address with its attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptStep {
    pub task: TaskId,
    pub iteration: u32,
    pub attempt: u32,
}

/// One tool call of an iteration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStep {
    pub task: TaskId,
    pub iteration: u32,
    pub attempt: u32,
    pub call_id: String,
}

/// A transition the workflow asks for (`set_state`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "op")]
pub enum StateOp {
    Cancel { reason: CancelReason },
    Complete { by: ParticipantId },
    Fail { reason: String },
    Retract { attempt: Attempt },
}

/// `set_state`'s input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateChange {
    pub task: TaskId,
    pub op: StateOp,
}

/// How the workflow ended; mirrors [`LoopOutcome`] without its payloads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "outcome")]
pub enum WorkflowOutcome {
    Completed,
    Canceled,
    Failed { reason: String },
}

fn app_error(err: impl std::fmt::Display) -> ActivityError {
    ActivityError::application(ApplicationFailure::new(err.to_string()))
}

/// The activities: the loop's phases over the agent, plus the transitions the workflow
/// decides. One instance per worker, holding the agent (and through it every client
/// with its credentials): nothing of it travels in a payload.
pub struct TaskActivities {
    pub agent: Arc<LocalAgent>,
}

impl core::fmt::Debug for TaskActivities {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TaskActivities").finish_non_exhaustive()
    }
}

/// Runs `work` unless the activity is cancelled first, in which case the task's
/// cancellation token stops in-flight tools and the activity reports cancelled.
async fn cancellable<T>(
    ctx: &ActivityContext,
    agent: &LocalAgent,
    task: &TaskId,
    work: impl Future<Output = Result<T, AgentError>>,
) -> Result<T, ActivityError> {
    tokio::select! {
        result = work => result.map_err(app_error),
        () = ctx.cancelled() => {
            agent.mailbox().token(task).cancel();
            Err(ActivityError::cancelled())
        }
    }
}

#[activities]
impl TaskActivities {
    /// Where the task is: its persisted iteration (the workflow continues from it after
    /// a restart, R14.4).
    #[activity]
    pub async fn progress(
        self: Arc<Self>,
        _ctx: ActivityContext,
        task: TaskId,
    ) -> Result<u32, ActivityError> {
        let record = self
            .agent
            .store()
            .get_task(&task)
            .await
            .map_err(app_error)?
            .ok_or_else(|| app_error(format!("task {task}: not found")))?;
        Ok(record.iteration)
    }

    /// Phase 1 (`build_context`).
    #[activity]
    pub async fn build_context(
        self: Arc<Self>,
        ctx: ActivityContext,
        step: IterationStep,
    ) -> Result<Begun, ActivityError> {
        let agent = Arc::clone(&self.agent);
        cancellable(&ctx, &agent, &step.task, async {
            agent.iteration_begin(&step.task, step.iteration).await
        })
        .await
    }

    /// Phase 2 (`call_model`).
    #[activity]
    pub async fn call_model(
        self: Arc<Self>,
        ctx: ActivityContext,
        step: AttemptStep,
    ) -> Result<ModelPhase, ActivityError> {
        let agent = Arc::clone(&self.agent);
        let attempt = Attempt {
            iteration: step.iteration,
            n: step.attempt,
        };
        cancellable(&ctx, &agent, &step.task, async {
            agent
                .iteration_model(&step.task, step.iteration, attempt)
                .await
        })
        .await
    }

    /// Phase 3 (`execute_tool`), one call.
    #[activity]
    pub async fn execute_tool(
        self: Arc<Self>,
        ctx: ActivityContext,
        step: ToolStep,
    ) -> Result<(), ActivityError> {
        let agent = Arc::clone(&self.agent);
        let attempt = Attempt {
            iteration: step.iteration,
            n: step.attempt,
        };
        cancellable(&ctx, &agent, &step.task, async {
            agent
                .iteration_tool(&step.task, step.iteration, attempt, &step.call_id)
                .await
        })
        .await
    }

    /// Phase 4 (`finish_iteration`).
    #[activity]
    pub async fn finish_iteration(
        self: Arc<Self>,
        ctx: ActivityContext,
        step: AttemptStep,
    ) -> Result<IterationOutcome, ActivityError> {
        let agent = Arc::clone(&self.agent);
        let attempt = Attempt {
            iteration: step.iteration,
            n: step.attempt,
        };
        cancellable(&ctx, &agent, &step.task, async {
            agent
                .iteration_finish(&step.task, step.iteration, attempt)
                .await
        })
        .await
    }

    /// `set_state`: a transition the workflow decided (R22.6) or a retraction (R22.4).
    #[activity]
    pub async fn set_state(
        self: Arc<Self>,
        _ctx: ActivityContext,
        change: StateChange,
    ) -> Result<(), ActivityError> {
        let agent = &self.agent;
        match change.op {
            StateOp::Cancel { reason } => {
                agent
                    .cancel(&change.task, reason)
                    .await
                    .map_err(app_error)?;
            }
            StateOp::Complete { by } => {
                agent.complete(&change.task, &by).await.map_err(app_error)?;
            }
            StateOp::Fail { reason } => {
                agent.fail(&change.task, &reason).await.map_err(app_error)?;
            }
            StateOp::Retract { attempt } => {
                agent
                    .retract(&change.task, attempt)
                    .await
                    .map_err(app_error)?;
            }
        }
        Ok(())
    }
}

/// One task's workflow: id = task id, started by the inbox, signalled by every later
/// event; its state is only what the signals delivered.
#[workflow]
#[derive(Debug, Default)]
pub struct TaskWorkflow {
    task: TaskId,
    retry: Option<RetrySchedule>,
    inbox_signals: u64,
    cancel: Option<CancelReason>,
    complete: Option<ParticipantId>,
    progress: Progress,
}

fn options(timeout: Duration) -> ActivityOptions {
    // Retries are orchestrated here, not by the engine (design.md § Durable execution).
    ActivityOptions::with_start_to_close_timeout(timeout)
        .retry_policy(
            temporalio_common::RetryPolicy::builder()
                .maximum_attempts(1)
                .build(),
        )
        .build()
}

const CONTEXT_TIMEOUT: Duration = Duration::from_secs(30);
const MODEL_TIMEOUT: Duration = Duration::from_secs(600);
const TOOL_TIMEOUT: Duration = Duration::from_secs(300);
const STATE_TIMEOUT: Duration = Duration::from_secs(30);

#[workflow_methods]
impl TaskWorkflow {
    #[init]
    pub fn new(_ctx: &WorkflowContextView, input: TaskWorkflowInput) -> Self {
        Self {
            task: input.task,
            retry: Some(input.retry),
            ..Self::default()
        }
    }

    /// The loop: phases until the task is terminal, waiting on signals when it waits.
    #[run]
    #[allow(clippy::too_many_lines)] // the loop body, read top to bottom
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<WorkflowOutcome> {
        let task = ctx.state(|s| s.task.clone());
        let retry = ctx.state(|s| s.retry.clone()).unwrap_or(RetrySchedule {
            initial_ms: 1000,
            coefficient: 2,
            max_interval_ms: 100_000,
            max_attempts: 5,
        });
        let mut iteration = ctx
            .execute_activity(
                TaskActivities::progress,
                task.clone(),
                options(STATE_TIMEOUT),
            )
            .await?;
        loop {
            // Stop signals first (R22.6).
            if let Some(stop) = Self::stop_signal(ctx) {
                let outcome = match &stop {
                    StateOp::Cancel { .. } => WorkflowOutcome::Canceled,
                    _ => WorkflowOutcome::Completed,
                };
                ctx.execute_activity(
                    TaskActivities::set_state,
                    StateChange {
                        task: task.clone(),
                        op: stop,
                    },
                    options(STATE_TIMEOUT),
                )
                .await?;
                return Ok(outcome);
            }
            let n = iteration + 1;
            ctx.state_mut(|s| {
                s.progress.iteration = n;
                s.progress.phase = "build_context".into();
                s.progress.attempt = 1;
            });
            let begun = ctx
                .execute_activity(
                    TaskActivities::build_context,
                    IterationStep {
                        task: task.clone(),
                        iteration: n,
                    },
                    options(CONTEXT_TIMEOUT),
                )
                .await?;
            match begun {
                Begun::Stopped { outcome } => match outcome {
                    LoopOutcome::Completed => return Ok(WorkflowOutcome::Completed),
                    LoopOutcome::Canceled => return Ok(WorkflowOutcome::Canceled),
                    LoopOutcome::Failed(reason) => {
                        return Ok(WorkflowOutcome::Failed { reason });
                    }
                    // Waiting for a turn, a participant or an operator: sleep until a
                    // signal, then look again.
                    LoopOutcome::AwaitingTurn
                    | LoopOutcome::AwaitingParticipant { .. }
                    | LoopOutcome::LimitReached { .. } => {
                        Self::wait_for_signal(ctx).await?;
                        continue;
                    }
                },
                Begun::Context { .. } => {}
            }

            // call_model, retried here under the schedule with a retraction first (R22.3–4).
            let mut attempt = 1;
            let model = loop {
                ctx.state_mut(|s| {
                    s.progress.phase = "call_model".into();
                    s.progress.attempt = attempt;
                });
                let result = ctx
                    .execute_activity(
                        TaskActivities::call_model,
                        AttemptStep {
                            task: task.clone(),
                            iteration: n,
                            attempt,
                        },
                        options(MODEL_TIMEOUT),
                    )
                    .await;
                match result {
                    Ok(phase) => break phase,
                    Err(ActivityExecutionError::Cancelled(_)) => {
                        return Self::stopped_by_signal(ctx, &task).await;
                    }
                    Err(err) => {
                        let Some(wait) = retry.backoff(attempt + 1) else {
                            ctx.execute_activity(
                                TaskActivities::set_state,
                                StateChange {
                                    task: task.clone(),
                                    op: StateOp::Fail {
                                        reason: format!(
                                            "model call failed after {attempt} attempts: {err}"
                                        ),
                                    },
                                },
                                options(STATE_TIMEOUT),
                            )
                            .await?;
                            return Ok(WorkflowOutcome::Failed {
                                reason: err.to_string(),
                            });
                        };
                        ctx.execute_activity(
                            TaskActivities::set_state,
                            StateChange {
                                task: task.clone(),
                                op: StateOp::Retract {
                                    attempt: Attempt {
                                        iteration: n,
                                        n: attempt,
                                    },
                                },
                            },
                            options(STATE_TIMEOUT),
                        )
                        .await?;
                        ctx.timer(wait).await;
                        attempt += 1;
                    }
                }
            };
            let calls = match model {
                ModelPhase::Stopped { outcome } => {
                    return Ok(match outcome {
                        LoopOutcome::Canceled => WorkflowOutcome::Canceled,
                        LoopOutcome::Completed => WorkflowOutcome::Completed,
                        LoopOutcome::Failed(reason) => WorkflowOutcome::Failed { reason },
                        other => WorkflowOutcome::Failed {
                            reason: format!("{other:?}"),
                        },
                    });
                }
                ModelPhase::Turn { calls, .. } => calls,
            };

            // execute_tool ×n, concurrently.
            ctx.state_mut(|s| s.progress.phase = "execute_tool".into());
            let tools = calls.iter().map(|call| {
                ctx.execute_activity(
                    TaskActivities::execute_tool,
                    ToolStep {
                        task: task.clone(),
                        iteration: n,
                        attempt,
                        call_id: call.clone(),
                    },
                    options(TOOL_TIMEOUT),
                )
            });
            for result in temporalio_workflow::join_all(tools).await {
                match result {
                    Ok(()) => {}
                    Err(ActivityExecutionError::Cancelled(_)) => {
                        return Self::stopped_by_signal(ctx, &task).await;
                    }
                    Err(err) => return Err(err.into()),
                }
            }

            // finish_iteration.
            ctx.state_mut(|s| s.progress.phase = "finish_iteration".into());
            let finished = ctx
                .execute_activity(
                    TaskActivities::finish_iteration,
                    AttemptStep {
                        task: task.clone(),
                        iteration: n,
                        attempt,
                    },
                    options(STATE_TIMEOUT),
                )
                .await?;
            iteration = n;
            match finished {
                IterationOutcome::Continue | IterationOutcome::Delivered => {}
                IterationOutcome::Suspended { .. } => Self::wait_for_signal(ctx).await?,
                IterationOutcome::Stopped { outcome } => {
                    return Ok(match outcome {
                        LoopOutcome::Canceled => WorkflowOutcome::Canceled,
                        LoopOutcome::Failed(reason) => WorkflowOutcome::Failed { reason },
                        _ => WorkflowOutcome::Completed,
                    });
                }
            }
        }
    }

    /// An inbox event arrived for the task; the event itself is in the store.
    #[signal]
    pub fn inbox_event(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _event: EventRef) {
        self.inbox_signals += 1;
        self.progress.inbox_signals = self.inbox_signals;
    }

    /// `CancelTask`.
    #[signal]
    pub fn cancel(&mut self, _ctx: &mut SyncWorkflowContext<Self>, reason: CancelReason) {
        self.cancel = Some(reason);
    }

    /// The `complete` transition.
    #[signal]
    pub fn complete(&mut self, _ctx: &mut SyncWorkflowContext<Self>, by: ParticipantId) {
        self.complete = Some(by);
    }

    /// Where the loop is.
    #[query]
    pub fn progress(&self, _ctx: &WorkflowContextView) -> Progress {
        self.progress.clone()
    }
}

/// The handle type a client uses for [`TaskWorkflow`] (the macro's run marker).
pub use task_workflow::Run as TaskWorkflowRun;

impl TaskWorkflow {
    fn stop_signal(ctx: &WorkflowContext<Self>) -> Option<StateOp> {
        ctx.state(|s| {
            if let Some(reason) = &s.cancel {
                Some(StateOp::Cancel {
                    reason: reason.clone(),
                })
            } else {
                s.complete
                    .as_ref()
                    .map(|by| StateOp::Complete { by: by.clone() })
            }
        })
    }

    /// Waits for the next signal of any kind.
    async fn wait_for_signal(ctx: &WorkflowContext<Self>) -> WorkflowResult<()> {
        let seen = ctx.state(|s| s.inbox_signals);
        ctx.wait_condition(|s| {
            s.inbox_signals > seen || s.cancel.is_some() || s.complete.is_some()
        })
        .await?;
        Ok(())
    }

    /// An activity was cancelled: the stop signal that caused it is applied.
    async fn stopped_by_signal(
        ctx: &WorkflowContext<Self>,
        task: &TaskId,
    ) -> WorkflowResult<WorkflowOutcome> {
        let op = Self::stop_signal(ctx).unwrap_or(StateOp::Cancel {
            reason: CancelReason::Policy {
                reason: "workflow.cancelled".into(),
            },
        });
        let outcome = match &op {
            StateOp::Complete { .. } => WorkflowOutcome::Completed,
            _ => WorkflowOutcome::Canceled,
        };
        ctx.execute_activity(
            TaskActivities::set_state,
            StateChange {
                task: task.clone(),
                op,
            },
            options(STATE_TIMEOUT),
        )
        .await?;
        Ok(outcome)
    }
}

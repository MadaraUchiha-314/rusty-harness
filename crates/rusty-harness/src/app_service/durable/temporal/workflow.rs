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

use crate::app_service::durable::{
    ActivityFailed, ActivityFailure, ActivityRef, ActivityRetry, ActivityRun, ActivityTimeouts,
    CancelReason, RetryDecision, RetryProposal,
};
use crate::app_service::inbox::EventRef;
use crate::harness::core_loop::agent::{
    AgentError, Begun, IterationOutcome, LocalAgent, LoopOutcome, ModelPhase,
};
use crate::harness::core_loop::task::{Attempt, ParticipantId, TaskId};
use crate::harness::hooks::HookCx;

/// What starts a task's workflow: the task and the retry schedule, so the body stays
/// deterministic without reading configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskWorkflowInput {
    pub task: TaskId,
    pub retry: RetrySchedule,
    #[serde(default)]
    pub timeouts: ActivityTimeouts,
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

/// The activities: the loop's phases over the agent, plus the transitions and the retry
/// decision the workflow asks for. One instance per worker, holding the agent (and
/// through it every client with its credentials): nothing of it travels in a payload.
pub struct TaskActivities {
    pub agent: Arc<LocalAgent>,
    /// How often a running activity heartbeats (R23.4): the harness heartbeat's interval.
    pub heartbeat_every: Duration,
}

impl core::fmt::Debug for TaskActivities {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TaskActivities")
            .field("heartbeat_every", &self.heartbeat_every)
            .finish_non_exhaustive()
    }
}

fn app_error(err: impl std::fmt::Display) -> ActivityError {
    ActivityError::application(ApplicationFailure::new(err.to_string()))
}

impl TaskActivities {
    fn cx(&self, task: &TaskId) -> HookCx {
        let mut cx = HookCx::new(
            Arc::new(crate::registry::Registry::new()),
            Arc::clone(self.agent.host()),
        );
        cx.task = Some(task.clone());
        cx
    }

    /// Runs one attempt under supervision: inside `durable.activity_run`, heartbeating
    /// every `heartbeat_every` (R23.4), and cancelled with the activity, in which case the
    /// task's token stops in-flight tools.
    async fn supervised<T>(
        &self,
        ctx: &ActivityContext,
        activity: ActivityRef,
        work: impl Future<Output = Result<T, AgentError>> + Send,
    ) -> Result<T, ActivityError>
    where
        T: Serialize + serde::de::DeserializeOwned,
    {
        let task = activity.task.clone();
        let cx = self.cx(&task);
        let every = self.heartbeat_every;
        let heartbeats = async {
            loop {
                tokio::time::sleep(every).await;
                let _ = ctx.record_heartbeat(()).await;
            }
        };
        let run = self
            .agent
            .hooks()
            .run::<ActivityRun, _, _>(&cx, activity, |_| async move {
                let value = work
                    .await
                    .map_err(|e| crate::HarnessError::Internal(e.to_string()))?;
                serde_json::to_value(value)
                    .map_err(|e| crate::HarnessError::Internal(e.to_string()))
            });
        tokio::select! {
            result = run => {
                let value = result.map_err(app_error)?;
                serde_json::from_value(value).map_err(app_error)
            }
            () = heartbeats => unreachable!("the heartbeat loop never ends"),
            () = ctx.cancelled() => {
                self.agent.mailbox().token(&task).cancel();
                Err(ActivityError::cancelled())
            }
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
        step: AttemptStep,
    ) -> Result<Begun, ActivityError> {
        let agent = Arc::clone(&self.agent);
        self.supervised(&ctx, step.activity("build_context"), async {
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
        self.supervised(&ctx, step.activity("call_model"), async {
            agent
                .iteration_model(&step.task, step.iteration, step.attempt())
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
        let activity = ActivityRef {
            task: step.task.clone(),
            iteration: step.iteration,
            attempt: step.attempt,
            activity: format!("execute_tool:{}", step.call_id),
        };
        let attempt = Attempt {
            iteration: step.iteration,
            n: step.attempt,
        };
        self.supervised(&ctx, activity, async {
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
        self.supervised(&ctx, step.activity("finish_iteration"), async {
            agent
                .iteration_finish(&step.task, step.iteration, step.attempt())
                .await
        })
        .await
    }

    /// `retry_decision` (R22.3): `durable.activity_failure`, then the schedule's proposal
    /// through `durable.activity_retry`; runs on whichever worker is alive, so a crashed
    /// worker's hooks still run.
    #[activity]
    pub async fn retry_decision(
        self: Arc<Self>,
        _ctx: ActivityContext,
        query: RetryQuery,
    ) -> Result<RetryDecision, ActivityError> {
        let cx = self.cx(&query.failure.activity.task);
        let hooks = self.agent.hooks();
        hooks
            .run::<ActivityFailed, _, _>(&cx, query.failure.clone(), |_| async { Ok(()) })
            .await
            .map_err(app_error)?;
        let proposed = match query.schedule.backoff(query.failure.activity.attempt + 1) {
            Some(after) => RetryDecision::Retry {
                after_ms: u64::try_from(after.as_millis()).unwrap_or(u64::MAX),
            },
            None => RetryDecision::Stop {
                reason: format!(
                    "{} failed after {} attempts: {}",
                    query.failure.activity.activity,
                    query.failure.activity.attempt,
                    query.failure.error
                ),
            },
        };
        hooks
            .run::<ActivityRetry, _, _>(
                &cx,
                RetryProposal {
                    failure: query.failure,
                    proposed,
                },
                |p| async move { Ok(p.proposed) },
            )
            .await
            .map_err(app_error)
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

/// `retry_decision`'s input: the failure and the schedule to decide under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryQuery {
    pub failure: ActivityFailure,
    pub schedule: RetrySchedule,
}

impl AttemptStep {
    fn attempt(&self) -> Attempt {
        Attempt {
            iteration: self.iteration,
            n: self.attempt,
        }
    }
    fn activity(&self, name: &str) -> ActivityRef {
        ActivityRef {
            task: self.task.clone(),
            iteration: self.iteration,
            attempt: self.attempt,
            activity: name.to_owned(),
        }
    }
}

/// One task's workflow: id = task id, started by the inbox, signalled by every later
/// event; its state is only what the signals delivered.
#[workflow]
#[derive(Debug, Default)]
pub struct TaskWorkflow {
    task: TaskId,
    retry: Option<RetrySchedule>,
    timeouts: ActivityTimeouts,
    inbox_signals: u64,
    cancel: Option<CancelReason>,
    complete: Option<ParticipantId>,
    progress: Progress,
}

/// Activity options (R22.5): a start-to-close bound, an optional heartbeat bound that
/// detects a crashed worker, and the engine's attempts fixed at one, because retries are
/// orchestrated here (design.md § Durable execution).
fn options(timeout_ms: u64, heartbeat_ms: Option<u64>) -> ActivityOptions {
    ActivityOptions::with_start_to_close_timeout(Duration::from_millis(timeout_ms))
        .maybe_heartbeat_timeout(heartbeat_ms.map(Duration::from_millis))
        .retry_policy(
            temporalio_common::RetryPolicy::builder()
                .maximum_attempts(1)
                .build(),
        )
        .build()
}

/// How one phase ended, after the retry orchestration.
enum Phase<T> {
    Done(T),
    Cancelled,
    Failed { reason: String },
}

#[workflow_methods]
impl TaskWorkflow {
    #[init]
    pub fn new(_ctx: &WorkflowContextView, input: TaskWorkflowInput) -> Self {
        Self {
            task: input.task,
            retry: Some(input.retry),
            timeouts: input.timeouts,
            ..Self::default()
        }
    }

    /// The loop: phases until the task is terminal, waiting on signals when it waits;
    /// every phase retried under the schedule (R22.3) with a retraction first (R22.4).
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
        let t = ctx.state(|s| s.timeouts.clone());
        let mut iteration = ctx
            .execute_activity(
                TaskActivities::progress,
                task.clone(),
                options(t.short_ms, None),
            )
            .await?;
        loop {
            // Stop signals first (R22.6).
            if let Some(stop) = Self::stop_signal(ctx) {
                return Self::apply_stop(ctx, &task, &t, stop).await;
            }
            let n = iteration + 1;
            let mut attempt: u32 = 1;
            // Signals counted from here on arrived after this iteration's boundary.
            let seen = ctx.state(|s| s.inbox_signals);
            ctx.state_mut(|s| {
                s.progress.iteration = n;
                s.progress.phase = "build_context".into();
                s.progress.attempt = attempt;
            });
            let begun = Self::retried(ctx, &task, &retry, &t, n, &mut attempt, |a| {
                ctx.execute_activity(
                    TaskActivities::build_context,
                    AttemptStep {
                        task: task.clone(),
                        iteration: n,
                        attempt: a,
                    },
                    options(t.short_ms, None),
                )
            })
            .await?;
            let begun = match begun {
                Phase::Done(b) => b,
                Phase::Cancelled => return Self::stopped_by_signal(ctx, &task, &t).await,
                Phase::Failed { reason } => return Ok(WorkflowOutcome::Failed { reason }),
            };
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
                        Self::wait_for_signal(ctx, seen).await?;
                        continue;
                    }
                },
                Begun::Context { .. } => {}
            }

            // call_model.
            ctx.state_mut(|s| s.progress.phase = "call_model".into());
            let model = Self::retried(ctx, &task, &retry, &t, n, &mut attempt, |a| {
                ctx.execute_activity(
                    TaskActivities::call_model,
                    AttemptStep {
                        task: task.clone(),
                        iteration: n,
                        attempt: a,
                    },
                    options(t.model_ms, Some(t.model_heartbeat_ms)),
                )
            })
            .await?;
            let calls = match model {
                Phase::Done(ModelPhase::Turn { calls, .. }) => calls,
                Phase::Done(ModelPhase::Stopped { outcome }) => {
                    return Ok(match outcome {
                        LoopOutcome::Canceled => WorkflowOutcome::Canceled,
                        LoopOutcome::Completed => WorkflowOutcome::Completed,
                        LoopOutcome::Failed(reason) => WorkflowOutcome::Failed { reason },
                        other => WorkflowOutcome::Failed {
                            reason: format!("{other:?}"),
                        },
                    });
                }
                Phase::Cancelled => return Self::stopped_by_signal(ctx, &task, &t).await,
                Phase::Failed { reason } => return Ok(WorkflowOutcome::Failed { reason }),
            };

            // execute_tool ×n, concurrently; the batch is retried as one when a call fails.
            ctx.state_mut(|s| s.progress.phase = "execute_tool".into());
            let (ctx_ref, task_ref, t_ref, calls_ref) = (&*ctx, &task, &t, &calls);
            let tools = Self::retried(ctx, &task, &retry, &t, n, &mut attempt, |a| {
                let runs = calls_ref.iter().map(move |call| {
                    ctx_ref.execute_activity(
                        TaskActivities::execute_tool,
                        ToolStep {
                            task: task_ref.clone(),
                            iteration: n,
                            attempt: a,
                            call_id: call.clone(),
                        },
                        options(t_ref.tool_ms, Some(t_ref.tool_heartbeat_ms)),
                    )
                });
                async move {
                    let mut outcome = Ok(());
                    for result in temporalio_workflow::join_all(runs).await {
                        if let Err(e) = result
                            && outcome.is_ok()
                        {
                            outcome = Err(e);
                        }
                    }
                    outcome
                }
            })
            .await?;
            match tools {
                Phase::Done(()) => {}
                Phase::Cancelled => return Self::stopped_by_signal(ctx, &task, &t).await,
                Phase::Failed { reason } => return Ok(WorkflowOutcome::Failed { reason }),
            }

            // finish_iteration.
            ctx.state_mut(|s| s.progress.phase = "finish_iteration".into());
            let finished = Self::retried(ctx, &task, &retry, &t, n, &mut attempt, |a| {
                ctx.execute_activity(
                    TaskActivities::finish_iteration,
                    AttemptStep {
                        task: task.clone(),
                        iteration: n,
                        attempt: a,
                    },
                    options(t.short_ms, None),
                )
            })
            .await?;
            iteration = n;
            match finished {
                Phase::Done(IterationOutcome::Continue | IterationOutcome::Delivered) => {}
                Phase::Done(IterationOutcome::Suspended { .. }) => {
                    Self::wait_for_signal(ctx, seen).await?;
                }
                Phase::Done(IterationOutcome::Stopped { outcome }) => {
                    return Ok(match outcome {
                        LoopOutcome::Canceled => WorkflowOutcome::Canceled,
                        LoopOutcome::Failed(reason) => WorkflowOutcome::Failed { reason },
                        _ => WorkflowOutcome::Completed,
                    });
                }
                Phase::Cancelled => return Self::stopped_by_signal(ctx, &task, &t).await,
                Phase::Failed { reason } => return Ok(WorkflowOutcome::Failed { reason }),
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

    /// Waits for a signal newer than `seen`: the count captured before the iteration's
    /// `build_context` applied the inbox, so a signal that arrived during the iteration
    /// is not lost (R20.4).
    async fn wait_for_signal(ctx: &WorkflowContext<Self>, seen: u64) -> WorkflowResult<()> {
        ctx.wait_condition(|s| {
            s.inbox_signals > seen || s.cancel.is_some() || s.complete.is_some()
        })
        .await?;
        Ok(())
    }

    /// Applies a stop signal through `set_state` and ends the workflow.
    async fn apply_stop(
        ctx: &WorkflowContext<Self>,
        task: &TaskId,
        t: &ActivityTimeouts,
        op: StateOp,
    ) -> WorkflowResult<WorkflowOutcome> {
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
            options(t.short_ms, None),
        )
        .await?;
        Ok(outcome)
    }

    /// An activity was cancelled: the stop signal that caused it is applied.
    async fn stopped_by_signal(
        ctx: &WorkflowContext<Self>,
        task: &TaskId,
        t: &ActivityTimeouts,
    ) -> WorkflowResult<WorkflowOutcome> {
        let op = Self::stop_signal(ctx).unwrap_or(StateOp::Cancel {
            reason: CancelReason::Policy {
                reason: "workflow.cancelled".into(),
            },
        });
        Self::apply_stop(ctx, task, t, op).await
    }

    /// Runs one phase as attempts under the schedule: a failure (the activity's own, or a
    /// timeout from a crashed worker) goes to `retry_decision`; a retry commits the
    /// retraction of the failed attempt first and sleeps the decided wait; a stop fails
    /// the task (R22.3–5).
    async fn retried<T, F, Fut>(
        ctx: &WorkflowContext<Self>,
        task: &TaskId,
        retry: &RetrySchedule,
        t: &ActivityTimeouts,
        iteration: u32,
        attempt: &mut u32,
        mut start: F,
    ) -> WorkflowResult<Phase<T>>
    where
        F: FnMut(u32) -> Fut,
        Fut: Future<Output = Result<T, ActivityExecutionError>>,
    {
        loop {
            ctx.state_mut(|s| s.progress.attempt = *attempt);
            let result = start(*attempt).await;
            let error = match result {
                Ok(value) => return Ok(Phase::Done(value)),
                Err(ActivityExecutionError::Cancelled(_)) => return Ok(Phase::Cancelled),
                Err(error) => error,
            };
            let phase = ctx.state(|s| s.progress.phase.clone());
            let failure = ActivityFailure {
                activity: ActivityRef {
                    task: task.clone(),
                    iteration,
                    attempt: *attempt,
                    activity: phase,
                },
                error: error.to_string(),
                timeout: error.as_timeout().is_some(),
            };
            let decision = ctx
                .execute_activity(
                    TaskActivities::retry_decision,
                    RetryQuery {
                        failure,
                        schedule: retry.clone(),
                    },
                    options(t.short_ms, None),
                )
                .await?;
            match decision {
                RetryDecision::Retry { after_ms } => {
                    ctx.execute_activity(
                        TaskActivities::set_state,
                        StateChange {
                            task: task.clone(),
                            op: StateOp::Retract {
                                attempt: Attempt {
                                    iteration,
                                    n: *attempt,
                                },
                            },
                        },
                        options(t.short_ms, None),
                    )
                    .await?;
                    ctx.timer(Duration::from_millis(after_ms)).await;
                    *attempt += 1;
                }
                RetryDecision::Stop { reason } => {
                    ctx.execute_activity(
                        TaskActivities::set_state,
                        StateChange {
                            task: task.clone(),
                            op: StateOp::Fail {
                                reason: reason.clone(),
                            },
                        },
                        options(t.short_ms, None),
                    )
                    .await?;
                    return Ok(Phase::Failed { reason });
                }
            }
        }
    }
}

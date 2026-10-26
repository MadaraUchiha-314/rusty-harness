//! `remote` and `embedded` as a [`DurableExecution`] (R22.1, R22.6, R14.4): one
//! `TaskWorkflow` per task, started or signalled by the inbox, cancelled and completed
//! through signals, resumed by id after a restart; the worker runs in this process.

use std::sync::{Arc, Mutex, PoisonError};

use temporalio_client::{WorkflowIdConflictPolicy, WorkflowSignalOptions, WorkflowStartOptions};
use temporalio_sdk::WorkerOptions;

use super::workflow::{RetrySchedule, TaskActivities, TaskWorkflow, TaskWorkflowInput};
use super::{TemporalConnection, TemporalWorker};
use crate::app_service::durable::ActivityTimeouts;
use crate::app_service::durable::{CancelReason, DurableError, DurableExecution, RetryPolicy};
use crate::app_service::inbox::EventRef;
use crate::config::DurableMode;
use crate::harness::core_loop::agent::LocalAgent;
use crate::harness::core_loop::task::{ParticipantId, TaskId};
use crate::harness::core_loop::task::{StateChange, StateChanged, TaskStateChange, Transition};
use crate::harness::hooks::{Hook, HookCx, HookError, HookOrder, PostHook};
use crate::harness_async_trait;
use crate::registry::{Entity, EntityId, EntityKind};
use std::time::Duration;

/// The Temporal modes.
pub struct TemporalExecution {
    id: EntityId,
    connection: Arc<TemporalConnection>,
    agent: Arc<LocalAgent>,
    retry: RetrySchedule,
    timeouts: ActivityTimeouts,
    heartbeat_every: Duration,
    workers: Mutex<Vec<TemporalWorker>>,
    signaller: std::sync::atomic::AtomicBool,
}

/// Tells a task's workflow about a cancel or complete decided anywhere in the harness (a
/// parent's terminal state cancelling its sub-tasks, a stall policy, an operator): the
/// record changed first; the workflow stops what it drives and returns (R22.6, R12.4).
struct WorkflowSignaller {
    connection: Arc<TemporalConnection>,
}

#[async_trait::async_trait]
impl PostHook<TaskStateChange> for WorkflowSignaller {
    async fn post(
        &self,
        _cx: &HookCx,
        input: &StateChange,
        output: StateChanged,
    ) -> Result<StateChanged, HookError> {
        let handle = self
            .connection
            .client()
            .get_workflow_handle::<super::workflow::TaskWorkflowRun>(input.task.id.clone());
        match &input.transition {
            Transition::Cancel => {
                let _ = handle
                    .signal(
                        TaskWorkflow::cancel,
                        CancelReason::Policy {
                            reason: "task.state_change".into(),
                        },
                        WorkflowSignalOptions::default(),
                    )
                    .await;
            }
            Transition::Complete => {
                let _ = handle
                    .signal(
                        TaskWorkflow::complete,
                        "service".to_owned(),
                        WorkflowSignalOptions::default(),
                    )
                    .await;
            }
            _ => {}
        }
        Ok(output)
    }
}

/// What a worker polls for. One process normally runs both; the chaos tests split them
/// to kill one side (R22.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerRoles {
    /// Workflow tasks and activity tasks.
    Both,
    /// Workflow tasks only.
    WorkflowsOnly,
    /// Activity tasks only.
    ActivitiesOnly,
}

impl core::fmt::Debug for TemporalExecution {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TemporalExecution")
            .field("mode", &self.connection.mode())
            .field("task_queue", &self.connection.task_queue())
            .finish_non_exhaustive()
    }
}

impl TemporalExecution {
    /// The mode over `connection`, driving `agent`'s phases as activities under `retry`.
    #[must_use]
    pub fn new(
        connection: Arc<TemporalConnection>,
        agent: Arc<LocalAgent>,
        retry: &RetryPolicy,
    ) -> Self {
        let id = match connection.mode() {
            DurableMode::Embedded => "durable/embedded",
            _ => "durable/remote",
        };
        Self {
            id: EntityId::new(id),
            connection,
            agent,
            retry: RetrySchedule::from(retry),
            timeouts: ActivityTimeouts::default(),
            heartbeat_every: Duration::from_secs(5),
            workers: Mutex::new(Vec::new()),
            signaller: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Per-activity timeouts other than the design's defaults (R22.5).
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: ActivityTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// How often a running activity heartbeats: the harness heartbeat's interval (R23.4).
    #[must_use]
    pub fn with_heartbeat_every(mut self, every: Duration) -> Self {
        self.heartbeat_every = every;
        self
    }

    /// Starts a worker for the task queue polling both task types.
    ///
    /// # Errors
    ///
    /// [`DurableError::Failed`] when the worker cannot be created.
    pub fn start_worker(&self) -> Result<(), DurableError> {
        self.start_worker_with(WorkerRoles::Both)
    }

    /// Starts a worker polling `roles`; several may run in one process.
    ///
    /// # Errors
    ///
    /// [`DurableError::Failed`] when the worker cannot be created.
    pub fn start_worker_with(&self, roles: WorkerRoles) -> Result<(), DurableError> {
        if !self
            .signaller
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            self.agent.hooks().register::<TaskStateChange>(
                EntityId::new(format!("{}.signaller", self.id.id)),
                Hook::Post(Arc::new(WorkflowSignaller {
                    connection: Arc::clone(&self.connection),
                })),
                HookOrder::default(),
            );
        }
        let builder = WorkerOptions::new(self.connection.task_queue().to_owned());
        let activities = TaskActivities {
            agent: Arc::clone(&self.agent),
            heartbeat_every: self.heartbeat_every,
        };
        let options = match roles {
            WorkerRoles::Both => builder
                .register_workflow::<TaskWorkflow>()
                .map_err(|e| DurableError::Failed(format!("register TaskWorkflow: {e}")))?
                .register_activities(activities)
                .build(),
            WorkerRoles::WorkflowsOnly => builder
                .register_workflow::<TaskWorkflow>()
                .map_err(|e| DurableError::Failed(format!("register TaskWorkflow: {e}")))?
                .build(),
            WorkerRoles::ActivitiesOnly => builder.register_activities(activities).build(),
        };
        let worker = TemporalWorker::start(&self.connection, options)?;
        self.workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(worker);
        Ok(())
    }

    /// Stops every worker this process runs.
    ///
    /// # Errors
    ///
    /// [`DurableError::Failed`] when a worker ended with an error.
    pub async fn stop_worker(&self) -> Result<(), DurableError> {
        let workers: Vec<TemporalWorker> =
            std::mem::take(&mut *self.workers.lock().unwrap_or_else(PoisonError::into_inner));
        for worker in workers {
            worker.stop().await?;
        }
        Ok(())
    }

    /// The connection.
    #[must_use]
    pub fn connection(&self) -> &Arc<TemporalConnection> {
        &self.connection
    }

    fn input(&self, task: &TaskId) -> TaskWorkflowInput {
        TaskWorkflowInput {
            task: task.clone(),
            retry: self.retry.clone(),
            timeouts: self.timeouts.clone(),
        }
    }

    fn start_options(&self, task: &TaskId) -> WorkflowStartOptions {
        WorkflowStartOptions::new(self.connection.task_queue().to_owned(), task.clone())
            .id_conflict_policy(WorkflowIdConflictPolicy::UseExisting)
            .build()
    }
}

impl Entity for TemporalExecution {
    fn kind(&self) -> EntityKind {
        EntityKind::DurableExecution
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl DurableExecution for TemporalExecution {
        async fn start_or_signal(&self, task: &TaskId, event: EventRef) -> Result<(), DurableError> {
            self.connection
                .client()
                .signal_with_start_workflow(
                    TaskWorkflow::run,
                    self.input(task),
                    TaskWorkflow::inbox_event,
                    event,
                    self.start_options(task),
                )
                .await
                .map(|_| ())
                .map_err(|e| DurableError::Failed(format!("start or signal {task}: {e}")))
        }

        async fn cancel(&self, task: &TaskId, reason: CancelReason) -> Result<(), DurableError> {
            // The record first (the caller sees `canceled` at once), then the workflow,
            // which cancels what it has in flight and returns.
            self.agent
                .cancel(task, reason.clone())
                .await
                .map_err(|e| DurableError::Failed(e.to_string()))?;
            let handle = self
                .connection
                .client()
                .get_workflow_handle::<super::workflow::TaskWorkflowRun>(task.clone());
            // Not running any more is not an error: the record already says canceled.
            let _ = handle
                .signal(TaskWorkflow::cancel, reason, WorkflowSignalOptions::default())
                .await;
            Ok(())
        }

        async fn complete(&self, task: &TaskId, by: ParticipantId) -> Result<(), DurableError> {
            self.agent
                .complete(task, &by)
                .await
                .map_err(|e| DurableError::Failed(e.to_string()))?;
            let handle = self
                .connection
                .client()
                .get_workflow_handle::<super::workflow::TaskWorkflowRun>(task.clone());
            let _ = handle
                .signal(TaskWorkflow::complete, by, WorkflowSignalOptions::default())
                .await;
            Ok(())
        }

        async fn resume_all(&self, tasks: Vec<TaskId>) -> Result<(), DurableError> {
            let mut failed = Vec::new();
            for task in tasks {
                if let Err(e) = self
                    .connection
                    .client()
                    .start_workflow(TaskWorkflow::run, self.input(&task), self.start_options(&task))
                    .await
                {
                    failed.push(format!("{task}: {e}"));
                }
            }
            if failed.is_empty() {
                Ok(())
            } else {
                Err(DurableError::Failed(format!("resume: {}", failed.join("; "))))
            }
        }

        fn mode(&self) -> DurableMode {
            self.connection.mode()
        }
    }
}

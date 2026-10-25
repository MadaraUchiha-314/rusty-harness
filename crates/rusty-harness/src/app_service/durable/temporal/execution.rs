//! `remote` and `embedded` as a [`DurableExecution`] (R22.1, R22.6, R14.4): one
//! `TaskWorkflow` per task, started or signalled by the inbox, cancelled and completed
//! through signals, resumed by id after a restart; the worker runs in this process.

use std::sync::{Arc, Mutex, PoisonError};

use temporalio_client::{WorkflowIdConflictPolicy, WorkflowSignalOptions, WorkflowStartOptions};
use temporalio_sdk::WorkerOptions;

use super::workflow::{RetrySchedule, TaskActivities, TaskWorkflow, TaskWorkflowInput};
use super::{TemporalConnection, TemporalWorker};
use crate::app_service::durable::{CancelReason, DurableError, DurableExecution, RetryPolicy};
use crate::app_service::inbox::EventRef;
use crate::config::DurableMode;
use crate::harness::core_loop::agent::LocalAgent;
use crate::harness::core_loop::task::{ParticipantId, TaskId};
use crate::harness_async_trait;
use crate::registry::{Entity, EntityId, EntityKind};

/// The Temporal modes.
pub struct TemporalExecution {
    id: EntityId,
    connection: Arc<TemporalConnection>,
    agent: Arc<LocalAgent>,
    retry: RetrySchedule,
    worker: Mutex<Option<TemporalWorker>>,
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
            worker: Mutex::new(None),
        }
    }

    /// Starts the worker for the task queue: `TaskWorkflow` and its activities.
    ///
    /// # Errors
    ///
    /// [`DurableError::Failed`] when the worker cannot be created.
    pub fn start_worker(&self) -> Result<(), DurableError> {
        let options = WorkerOptions::new(self.connection.task_queue().to_owned())
            .register_workflow::<TaskWorkflow>()
            .map_err(|e| DurableError::Failed(format!("register TaskWorkflow: {e}")))?
            .register_activities(TaskActivities {
                agent: Arc::clone(&self.agent),
            })
            .build();
        let worker = TemporalWorker::start(&self.connection, options)?;
        *self.worker.lock().unwrap_or_else(PoisonError::into_inner) = Some(worker);
        Ok(())
    }

    /// Stops the worker, when one runs.
    ///
    /// # Errors
    ///
    /// [`DurableError::Failed`] when the worker ended with an error.
    pub async fn stop_worker(&self) -> Result<(), DurableError> {
        let worker = self
            .worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match worker {
            Some(worker) => worker.stop().await,
            None => Ok(()),
        }
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

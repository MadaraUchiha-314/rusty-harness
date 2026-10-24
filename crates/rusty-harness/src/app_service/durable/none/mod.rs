//! Durable mode `none`: the same loop and retry policy in process, with no durability
//! guarantee (R22.7).
//!
//! [`InProcessExecution`] runs [`LocalAgent::run`] on the runtime seam, one task at a
//! time per task id; signals are the agent's in-memory mailbox; an infrastructure
//! failure of the loop is retried under the configured [`RetryPolicy`]. On restart the
//! harness lists the store's non-terminal tasks and hands them to `resume_all`, and the
//! loop restarts at the last persisted iteration (R14.4).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use super::{CancelReason, DurableError, DurableExecution, RetryPolicy};
use crate::app_service::inbox::{EventRef, EventTarget};
use crate::config::DurableMode;
use crate::harness::core_loop::agent::{LocalAgent, LoopOutcome};
use crate::harness::core_loop::task::{ParticipantId, TaskId};
use crate::harness_async_trait;
use crate::registry::{Entity, EntityId, EntityKind};
use crate::runtime::Runtime;

/// The in-process mode.
pub struct InProcessExecution<R: Runtime> {
    id: EntityId,
    agent: Arc<LocalAgent>,
    runtime: Arc<R>,
    retry: RetryPolicy,
    running: Arc<Mutex<BTreeSet<TaskId>>>,
}

impl<R: Runtime> core::fmt::Debug for InProcessExecution<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InProcessExecution")
            .field(
                "running",
                &self
                    .running
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .len(),
            )
            .finish_non_exhaustive()
    }
}

impl<R: Runtime + MaybeSendSync + 'static> InProcessExecution<R> {
    /// The mode over `agent`, spawning on `runtime`.
    #[must_use]
    pub fn new(agent: Arc<LocalAgent>, runtime: Arc<R>, retry: RetryPolicy) -> Self {
        Self {
            id: EntityId::new("durable/none"),
            agent,
            runtime,
            retry,
            running: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    /// Whether the mode is driving `task` right now.
    #[must_use]
    pub fn is_running(&self, task: &TaskId) -> bool {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(task)
    }

    /// Runs the loop for `task` to its next waiting point, retrying an infrastructure
    /// failure under the policy; the loop's own outcomes are final.
    pub async fn drive(
        agent: &LocalAgent,
        runtime: &R,
        retry: &RetryPolicy,
        task: &TaskId,
    ) -> LoopOutcome {
        let mut attempt = 1;
        loop {
            match agent.run(task).await {
                Ok(outcome) => return outcome,
                Err(error) => {
                    attempt += 1;
                    match retry.backoff(attempt) {
                        Some(wait) => runtime.sleep(wait).await,
                        None => return LoopOutcome::Failed(error.to_string()),
                    }
                }
            }
        }
    }

    fn spawn(&self, task: TaskId) {
        {
            let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
            if !running.insert(task.clone()) {
                return; // idempotent on the task id
            }
        }
        let agent = Arc::clone(&self.agent);
        let runtime = Arc::clone(&self.runtime);
        let retry = self.retry.clone();
        let running = Arc::clone(&self.running);
        let rt = Arc::clone(&self.runtime);
        rt.spawn(async move {
            let _ = Self::drive(&agent, &runtime, &retry, &task).await;
            running
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&task);
        });
    }
}

/// `Send + Sync` natively, nothing on `wasm32`; what a runtime shared by spawned tasks needs.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSendSync: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync> MaybeSendSync for T {}
/// `Send + Sync` natively, nothing on `wasm32`; what a runtime shared by spawned tasks needs.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSendSync {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSendSync for T {}

impl<R: Runtime + MaybeSendSync + 'static> Entity for InProcessExecution<R> {
    fn kind(&self) -> EntityKind {
        EntityKind::DurableExecution
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl<R: Runtime + MaybeSendSync + 'static> DurableExecution for InProcessExecution<R> {
        async fn start_or_signal(&self, task: &TaskId, event: EventRef) -> Result<(), DurableError> {
            match event.target {
                EventTarget::Task(_) | EventTarget::Context(_) => {}
            }
            if !self.is_running(task) {
                self.spawn(task.clone());
            }
            Ok(())
        }

        async fn cancel(&self, task: &TaskId, reason: CancelReason) -> Result<(), DurableError> {
            self.agent
                .cancel(task, reason)
                .await
                .map(|_| ())
                .map_err(|e| DurableError::Failed(e.to_string()))
        }

        async fn complete(&self, task: &TaskId, by: ParticipantId) -> Result<(), DurableError> {
            self.agent
                .complete(task, &by)
                .await
                .map(|_| ())
                .map_err(|e| DurableError::Failed(e.to_string()))
        }

        async fn resume_all(&self, tasks: Vec<TaskId>) -> Result<(), DurableError> {
            for task in tasks {
                self.spawn(task);
            }
            Ok(())
        }

        fn mode(&self) -> DurableMode {
            DurableMode::None
        }
    }
}

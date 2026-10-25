//! The smallest workflow and activity pair: proof that a connection and a worker work end
//! to end (the `embedded` scenario). The SDK's macros generate items this crate's lints
//! cannot document, hence the module-level allowances.
#![allow(
    missing_docs,
    missing_debug_implementations,
    clippy::missing_errors_doc,
    clippy::unused_async // activities are `async fn` by the macro's contract
)]

use std::sync::Arc;
use std::time::Duration;

use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use temporalio_sdk::{ActivityOptions, WorkerOptions, WorkflowContext, WorkflowResult};

use super::{DurableError, TemporalConnection};

/// The smallest workflow: one activity that answers. It proves a connection and a worker
/// work end to end (the `embedded` scenario) and carries nothing but a string.
#[workflow]
#[derive(Debug, Default)]
pub struct PingWorkflow;

#[workflow_methods]
impl PingWorkflow {
    /// Runs the `pong` activity once.
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, name: String) -> WorkflowResult<String> {
        let answer = ctx
            .execute_activity(
                PingActivities::pong,
                name,
                ActivityOptions::start_to_close_timeout(Duration::from_secs(10)),
            )
            .await?;
        Ok(answer)
    }
}

/// The activities behind [`PingWorkflow`]. The instance may hold a resolved secret (as a
/// real activity set will hold a model client); it never appears in a payload.
pub struct PingActivities {
    /// A value the activities may use and must never return.
    pub held_secret: Option<Arc<crate::host::Secret<String>>>,
}

impl core::fmt::Debug for PingActivities {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PingActivities")
            .field("holds_secret", &self.held_secret.is_some())
            .finish()
    }
}

#[activities]
impl PingActivities {
    /// Answers `name`, and says whether it holds a secret, without saying which.
    #[activity]
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)] // `async fn` by the macro's contract
    pub async fn pong(
        self: Arc<Self>,
        _ctx: ActivityContext,
        name: String,
    ) -> Result<String, ActivityError> {
        let held = if self.held_secret.is_some() {
            " (holding a credential)"
        } else {
            ""
        };
        Ok(format!("pong: {name}{held}"))
    }
}

/// Worker options registering the ping pair on `connection`'s task queue.
///
/// # Errors
///
/// [`DurableError::Failed`] when the workflow cannot be registered.
pub fn ping_worker_options(
    connection: &TemporalConnection,
    activities: PingActivities,
) -> Result<WorkerOptions, DurableError> {
    Ok(WorkerOptions::new(connection.task_queue.clone())
        .register_workflow::<PingWorkflow>()
        .map_err(|e| DurableError::Failed(format!("register PingWorkflow: {e}")))?
        .register_activities(activities)
        .build())
}

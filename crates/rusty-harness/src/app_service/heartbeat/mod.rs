//! The heartbeat service: liveness, event re-signalling, stalled-task detection (R23).
//!
//! Every `[loop] heartbeat_interval` the [`Heartbeat`] ticks: it re-signals inbox events
//! still `Received`, or `Signalled` longer than `[loop] signal_grace` without being
//! applied (R23.1, R20.5); writes a [`Liveness`] record for the service layer and emits
//! it as a gauge event for o11y (R23.2); and runs `task.stalled` for every working task
//! whose record has not changed within `[loop] stall_after`, acting on the hook's
//! [`StallAction`], a warning by default (R23.3). In the Temporal modes the long
//! activities heartbeat the engine on the same interval (R23.4, task 30).

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::HarnessError;
use crate::app_service::durable::{CancelReason, DurableError, DurableExecution};
use crate::app_service::inbox::{EventRef, InboxStatus};
use crate::harness::core_loop::agent::LocalAgent;
use crate::harness::core_loop::persistence::{PersistenceStore, StoreError, TaskQuery};
use crate::harness::core_loop::task::TaskId;
use crate::harness::hooks::{HookCx, HookDispatcher, HookPoint, Operation};
use crate::runtime::Runtime;

/// What the service layer and o11y see of the agent (R23.2); stored under
/// `liveness/<worker_id>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Liveness {
    /// This process.
    pub worker_id: String,
    /// When the loop last reached a boundary, if it ever did.
    pub last_boundary: Option<DateTime<Utc>>,
    /// The tasks a loop is running right now.
    pub running_tasks: Vec<TaskId>,
    /// Open tasks in the store.
    pub open_tasks: u64,
    /// When this record was written.
    pub at: DateTime<Utc>,
}

impl Liveness {
    /// The store key.
    #[must_use]
    pub fn key(worker_id: &str) -> String {
        format!("liveness/{worker_id}")
    }
}

/// A task that has not progressed within the bound (R23.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StalledTask {
    /// The task.
    pub task: TaskId,
    /// When it last changed.
    pub since: DateTime<Utc>,
    /// How long ago, in seconds.
    pub idle_secs: u64,
}

/// What to do about a stalled task; the hook decides, `Warn` is the default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "action")]
pub enum StallAction {
    /// Emit a warning event and leave the task alone.
    #[default]
    Warn,
    /// Cancel the task through durable execution.
    Cancel,
    /// Escalate: a warning carrying `reason` for whoever watches the trace.
    Escalate {
        /// A string-catalog key or free text for the trace.
        reason: String,
    },
}

/// The `heartbeat.tick` operation: the liveness goes in, hooks observe it.
#[derive(Debug)]
pub struct HeartbeatTick;

impl Operation for HeartbeatTick {
    const POINT: HookPoint = HookPoint::HeartbeatTick;
    type Input = Liveness;
    type Output = ();
    fn attributes(input: &Liveness, (): &()) -> Vec<(&'static str, String)> {
        vec![
            ("running_tasks", input.running_tasks.len().to_string()),
            ("open_tasks", input.open_tasks.to_string()),
        ]
    }
}

/// The `task.stalled` operation (R23.3).
#[derive(Debug)]
pub struct TaskStalled;

impl Operation for TaskStalled {
    const POINT: HookPoint = HookPoint::TaskStalled;
    type Input = StalledTask;
    type Output = StallAction;
    fn identity(input: &StalledTask) -> String {
        input.task.clone()
    }
}

/// Why a tick failed.
#[derive(Debug, thiserror::Error)]
pub enum HeartbeatError {
    /// The store.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Durable execution.
    #[error(transparent)]
    Durable(#[from] DurableError),
    /// A hook.
    #[error(transparent)]
    Harness(#[from] HarnessError),
}

/// What one tick did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// Events re-signalled.
    pub forwarded: Vec<String>,
    /// Tasks found stalled, with the action taken.
    pub stalled: Vec<(TaskId, StallAction)>,
    /// The liveness written.
    pub liveness: Option<Liveness>,
}

/// The heartbeat (R23).
pub struct Heartbeat {
    worker_id: String,
    interval: Duration,
    signal_grace: Duration,
    stall_after: Duration,
    agent: Arc<LocalAgent>,
    store: Arc<dyn PersistenceStore>,
    durable: Arc<dyn DurableExecution>,
    dispatcher: Arc<HookDispatcher>,
}

impl core::fmt::Debug for Heartbeat {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Heartbeat")
            .field("worker_id", &self.worker_id)
            .field("interval", &self.interval)
            .finish_non_exhaustive()
    }
}

impl Heartbeat {
    /// A heartbeat for `agent`, from `[loop]`'s knobs.
    #[must_use]
    pub fn new(
        worker_id: impl Into<String>,
        config: &crate::config::LoopConfig,
        agent: Arc<LocalAgent>,
        durable: Arc<dyn DurableExecution>,
        dispatcher: Arc<HookDispatcher>,
    ) -> Self {
        Self {
            worker_id: worker_id.into(),
            interval: config.heartbeat_interval.into(),
            signal_grace: config.signal_grace.into(),
            stall_after: config.stall_after.into(),
            store: Arc::clone(agent.store()),
            agent,
            durable,
            dispatcher,
        }
    }

    /// The interval.
    #[must_use]
    pub fn interval(&self) -> Duration {
        self.interval
    }

    fn now(&self) -> DateTime<Utc> {
        self.agent
            .host()
            .clock()
            .map_or_else(|_| Utc::now(), |c| c.now())
    }

    /// One tick: forward, record liveness, detect stalls.
    ///
    /// # Errors
    ///
    /// [`HeartbeatError`]; a failing hook under a log-and-continue policy does not fail
    /// the tick.
    #[allow(clippy::too_many_lines)] // the three duties of one tick, read top to bottom
    pub async fn tick(&self) -> Result<TickReport, HeartbeatError> {
        let now = self.now();
        let mut report = TickReport::default();
        let mut open_query = TaskQuery::everyone();
        open_query.open_only = true;
        open_query.limit = usize::MAX;
        let open = self.store.list_tasks(open_query).await?.items;

        // R23.1 / R20.5: events whose signal was lost or never answered.
        for task in &open {
            for event in self.store.pending_inbox(&task.id).await? {
                let stale = match &event.status {
                    InboxStatus::Received => true,
                    InboxStatus::Signalled { at } => now
                        .signed_duration_since(*at)
                        .to_std()
                        .is_ok_and(|d| d >= self.signal_grace),
                    InboxStatus::Applied { .. } => false,
                };
                if !stale {
                    continue;
                }
                let reference = EventRef {
                    id: event.id.clone(),
                    target: event.target.clone(),
                };
                self.durable
                    .start_or_signal(&task.id, reference.clone())
                    .await?;
                self.store
                    .mark_inbox(&reference, InboxStatus::Signalled { at: now })
                    .await?;
                report.forwarded.push(event.id.clone());
            }
        }

        // R23.2: liveness for the service layer and the o11y gauge.
        let running: Vec<TaskId> = open
            .iter()
            .filter(|t| self.agent.mailbox().is_running(&t.id))
            .map(|t| t.id.clone())
            .collect();
        let liveness = Liveness {
            worker_id: self.worker_id.clone(),
            last_boundary: self.agent.last_boundary(),
            running_tasks: running,
            open_tasks: open.len() as u64,
            at: now,
        };
        self.store
            .put_blob(
                &Liveness::key(&self.worker_id),
                bytes::Bytes::from(serde_json::to_vec(&liveness).unwrap_or_default()),
            )
            .await?;
        tracing::info!(
            target: "rusty_harness.heartbeat",
            worker = %self.worker_id,
            running_tasks = liveness.running_tasks.len(),
            open_tasks = liveness.open_tasks,
            "heartbeat"
        );
        let cx = HookCx::new(
            Arc::new(crate::registry::Registry::new()),
            Arc::clone(self.agent.host()),
        );
        self.dispatcher
            .run::<HeartbeatTick, _, _>(&cx, liveness.clone(), |_| async { Ok(()) })
            .await?;
        report.liveness = Some(liveness);

        // R23.3: tasks that stopped progressing.
        for task in open.iter().filter(|t| t.state == a2a::TaskState::Working) {
            let idle = now
                .signed_duration_since(task.updated_at)
                .to_std()
                .unwrap_or_default();
            if idle < self.stall_after {
                continue;
            }
            let stalled = StalledTask {
                task: task.id.clone(),
                since: task.updated_at,
                idle_secs: idle.as_secs(),
            };
            let mut cx = cx.clone();
            cx.task = Some(task.id.clone());
            let action = self
                .dispatcher
                .run::<TaskStalled, _, _>(&cx, stalled, |_| async { Ok(StallAction::Warn) })
                .await?;
            match &action {
                StallAction::Warn => {
                    tracing::warn!(target: "rusty_harness.heartbeat", task = %task.id, idle_secs = idle.as_secs(), "task stalled");
                }
                StallAction::Escalate { reason } => {
                    tracing::warn!(target: "rusty_harness.heartbeat", task = %task.id, idle_secs = idle.as_secs(), reason = %reason, "task stalled; escalated");
                }
                StallAction::Cancel => {
                    self.durable
                        .cancel(
                            &task.id,
                            CancelReason::Policy {
                                reason: "task.stalled".into(),
                            },
                        )
                        .await?;
                }
            }
            report.stalled.push((task.id.clone(), action));
        }
        Ok(report)
    }

    /// Ticks forever on `runtime`, every interval; errors are logged and the beat goes on.
    pub fn start<R: Runtime + Send + Sync + 'static>(self: Arc<Self>, runtime: Arc<R>) {
        let rt = Arc::clone(&runtime);
        rt.spawn(async move {
            loop {
                runtime.sleep(self.interval).await;
                if let Err(e) = self.tick().await {
                    tracing::warn!(target: "rusty_harness.heartbeat", error = %e, "heartbeat tick failed");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{Heartbeat, Liveness, StallAction, TaskStalled};
    use crate::app_service::durable::{CancelReason, DurableError, DurableExecution};
    use crate::app_service::inbox::EventRef;
    use crate::assets::PromptRegistry;
    use crate::config::{Config, DurableMode};
    use crate::harness::commands::{ModelRouter, ModelSelection};
    use crate::harness::core_loop::agent::{LocalAgent, LocalAgentParts};
    use crate::harness::core_loop::persistence::{MemoryStore, TaskCommit};
    use crate::harness::core_loop::task::{
        ParticipantId, TaskId, TaskMode, TaskRecord, Transition,
    };
    use crate::harness::hooks::{
        GuardHook, Hook, HookCx, HookDispatcher, HookError, HookOrder, InHook, Next,
    };
    use crate::harness::models::llm::Llm;
    use crate::harness::models::llm::testing::RecordedLlm;
    use crate::harness_async_trait;
    use crate::host::Host;
    use crate::host::testing::FixedClock;
    use crate::registry::{Entity, EntityId, EntityKind, Registry};
    use chrono::{TimeZone, Utc};
    use std::sync::{Arc, Mutex};

    /// Records what the heartbeat asked of durable execution.
    #[derive(Default)]
    struct Fake {
        id: Option<EntityId>,
        signals: Mutex<Vec<(TaskId, String)>>,
        cancels: Mutex<Vec<TaskId>>,
    }
    impl Entity for Fake {
        fn kind(&self) -> EntityKind {
            EntityKind::DurableExecution
        }
        fn entity_id(&self) -> &EntityId {
            self.id.as_ref().unwrap()
        }
    }
    harness_async_trait! {
        impl DurableExecution for Fake {
            async fn start_or_signal(&self, task: &TaskId, event: EventRef) -> Result<(), DurableError> {
                self.signals.lock().unwrap().push((task.clone(), event.id));
                Ok(())
            }
            async fn cancel(&self, task: &TaskId, _reason: CancelReason) -> Result<(), DurableError> {
                self.cancels.lock().unwrap().push(task.clone());
                Ok(())
            }
            async fn complete(&self, _task: &TaskId, _by: ParticipantId) -> Result<(), DurableError> {
                Ok(())
            }
            async fn resume_all(&self, _tasks: Vec<TaskId>) -> Result<(), DurableError> {
                Ok(())
            }
            fn mode(&self) -> DurableMode {
                DurableMode::None
            }
        }
    }

    fn rig(clock: Arc<FixedClock>) -> (Heartbeat, Arc<Fake>, Arc<HookDispatcher>, Arc<LocalAgent>) {
        let registry = Arc::new(Registry::new());
        let llm: Arc<dyn Llm> = Arc::new(RecordedLlm::new("default", vec![]));
        registry.register(llm).unwrap();
        let host = Arc::new(Host::builder().clock(clock).build());
        let dispatcher = Arc::new(HookDispatcher::new());
        let agent = Arc::new(LocalAgent::new(LocalAgentParts {
            config: Config::default(),
            registry: Arc::clone(&registry),
            dispatcher: Arc::clone(&dispatcher),
            host,
            store: Arc::new(MemoryStore::new()),
            prompts: Arc::new(PromptRegistry::builtin()),
            router: Arc::new(ModelRouter::new(
                registry,
                ModelSelection {
                    model: "default".into(),
                    effort: None,
                },
            )),
        }));
        let fake = Arc::new(Fake {
            id: Some(EntityId::new("fake")),
            ..Fake::default()
        });
        let durable: Arc<dyn DurableExecution> = fake.clone();
        let hb = Heartbeat::new(
            "w1",
            &Config::default().r#loop,
            Arc::clone(&agent),
            durable,
            Arc::clone(&dispatcher),
        );
        (hb, fake, dispatcher, agent)
    }

    #[tokio::test]
    async fn a_task_idle_past_stall_after_runs_the_stalled_hook_and_the_default_is_a_warning() {
        struct Record(Arc<Mutex<Vec<u64>>>);
        harness_async_trait! {
            impl GuardHook<TaskStalled> for Record {
                async fn guard(&self, _cx: &HookCx, input: &super::StalledTask) -> Result<crate::harness::hooks::GuardOutcome, HookError> {
                    self.0.lock().unwrap().push(input.idle_secs);
                    Ok(crate::harness::hooks::GuardOutcome::Allow)
                }
            }
        }

        struct CancelIt;
        harness_async_trait! {
            impl InHook<TaskStalled> for CancelIt {
                async fn run<'a>(&self, _cx: &'a HookCx, _input: super::StalledTask, _next: Next<'a, TaskStalled>) -> Result<StallAction, crate::HarnessError> {
                    Ok(StallAction::Cancel)
                }
            }
        }

        let t0 = Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap();
        let clock = Arc::new(FixedClock::at(t0));
        let (hb, fake, dispatcher, agent) = rig(Arc::clone(&clock));
        let mut task = TaskRecord::new(
            crate::harness::core_loop::task::tests::draft("t1", TaskMode::LongRunning),
            t0,
        );
        task.transition(&Transition::Start, t0).unwrap();
        agent.store().commit(TaskCommit::of(task)).await.unwrap();
        let fresh = hb.tick().await.unwrap();
        assert!(fresh.stalled.is_empty(), "just started");
        assert_eq!(fresh.liveness.as_ref().unwrap().open_tasks, 1);
        let stored: Liveness = serde_json::from_slice(
            &agent
                .store()
                .get_blob(&Liveness::key("w1"))
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(stored.worker_id, "w1");
        assert_eq!(stored.at, t0);

        clock.advance(chrono::Duration::minutes(16));
        let seen = Arc::new(Mutex::new(Vec::new()));
        dispatcher.register::<TaskStalled>(
            EntityId::new("record"),
            Hook::Guard(Arc::new(Record(Arc::clone(&seen)))),
            HookOrder::default(),
        );
        let late = hb.tick().await.unwrap();
        assert_eq!(late.stalled, [("t1".to_owned(), StallAction::Warn)]);
        assert_eq!(seen.lock().unwrap().as_slice(), [16 * 60]);
        assert!(
            fake.cancels.lock().unwrap().is_empty(),
            "a warning cancels nothing"
        );

        // A hook that replaces the decision cancels through durable execution.
        dispatcher.register::<TaskStalled>(
            EntityId::new("cancel-it"),
            Hook::In(Arc::new(CancelIt)),
            HookOrder::default(),
        );
        let decided = hb.tick().await.unwrap();
        assert_eq!(decided.stalled, [("t1".to_owned(), StallAction::Cancel)]);
        assert_eq!(fake.cancels.lock().unwrap().as_slice(), ["t1"]);
    }
}

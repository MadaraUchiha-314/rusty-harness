//! The durable inbox: `InboxEvent`, `Inbox`, delivery policy and the
//! received/signalled/applied states (R20).
//!
//! This module holds the event types the store persists (task 14); the `Inbox` service
//! that validates, accepts and drains them arrives with the service layer (task 21).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::harness::core_loop::task::{Caller, ContextId, TaskId};

/// An inbox event id (a UUID string chosen by the sender or minted on acceptance).
pub type EventId = String;

/// Whom an event addresses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "id")]
pub enum EventTarget {
    /// A task.
    Task(TaskId),
    /// A context; a `Message` with no task creates one (R20.6).
    Context(ContextId),
}

/// Where an accepted event is (design.md § Inbox: the tri-state).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum InboxStatus {
    /// Stored, not yet handed to the workflow.
    Received,
    /// Handed to durable execution.
    Signalled {
        /// When.
        at: DateTime<Utc>,
    },
    /// Folded into the task by a commit; written only by `commit(inbox_applied)`.
    Applied {
        /// The commit that applied it.
        commit: u64,
    },
}

/// Interrupt at the next loop boundary, or wait for the current step (R20.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeliveryKind {
    /// Deliver at the next loop boundary.
    #[default]
    Interrupt,
    /// Queue until the current step completes.
    Queue,
}

/// An accepted A2A event (R20.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InboxEvent {
    /// The id.
    pub id: EventId,
    /// Whom it addresses.
    pub target: EventTarget,
    /// The event, in the shape A2A streams and pushes it.
    pub kind: a2a::StreamResponse,
    /// Where it is.
    pub status: InboxStatus,
    /// Who sent it.
    pub sender: Caller,
    /// When it arrived.
    pub received_at: DateTime<Utc>,
    /// How it is delivered to a running task.
    #[serde(default)]
    pub delivery: DeliveryKind,
    /// Extension URIs the sender activated.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// The W3C `traceparent` it arrived with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
}

/// A stored event's reference: enough to signal a workflow with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRef {
    /// The event.
    pub id: EventId,
    /// Whom it addresses.
    pub target: EventTarget,
}

use std::sync::Arc;

use crate::HarnessError;
use crate::app_service::durable::{DurableError, DurableExecution};
use crate::harness::core_loop::agent::{AgentError, AgentRequest, CallContext, LocalAgent};
use crate::harness::core_loop::persistence::{PersistenceStore, StoreError};
use crate::harness::core_loop::task::TASK_EXT;
use crate::harness::hooks::{HookCx, HookDispatcher, HookPoint, Operation};

/// Why the inbox refused an event (R20.6); every variant maps to the protocol's error.
#[derive(Debug, thiserror::Error)]
pub enum InboxError {
    /// The task does not exist (`TaskNotFoundError`).
    #[error("task {0}: not found")]
    NotFound(TaskId),
    /// The task is terminal (`UnsupportedOperationError`, R21.6).
    #[error("task {0}: terminal; start a new task in the same context with referenceTaskIds")]
    Terminal(TaskId),
    /// The message names a context the task does not belong to.
    #[error("task {task} belongs to context {expected}, not {got}")]
    ContextMismatch {
        /// The task.
        task: TaskId,
        /// Its context.
        expected: ContextId,
        /// What the message said.
        got: ContextId,
    },
    /// The event cannot be applied to its target (an update addressed to a context).
    #[error("invalid inbox event: {0}")]
    Invalid(String),
    /// A hook vetoed or failed (fail-closed).
    #[error(transparent)]
    Harness(#[from] HarnessError),
    /// The store.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Durable execution.
    #[error(transparent)]
    Durable(#[from] DurableError),
    /// The agent refused the message.
    #[error(transparent)]
    Agent(#[from] AgentError),
}

impl InboxError {
    /// The A2A error for the wire (R19.4, R20.6).
    #[must_use]
    pub fn to_a2a(&self) -> a2a::A2AError {
        match self {
            Self::NotFound(t) => a2a::A2AError::task_not_found(t),
            Self::Terminal(t) => {
                a2a::A2AError::unsupported_operation(format!("task {t} is terminal"))
            }
            Self::ContextMismatch { .. } | Self::Invalid(_) => {
                a2a::A2AError::invalid_params(self.to_string())
            }
            Self::Harness(e) => e.to_a2a(),
            Self::Agent(e) => e.to_a2a(),
            Self::Store(_) | Self::Durable(_) => a2a::A2AError::internal(self.to_string()),
        }
    }
}

/// The delivery policy (R20.4): a participant's message interrupts at the next loop
/// boundary; a sub-agent's status or artifact update queues until the step that awaits
/// it. `inbox.deliver` hooks may reclassify.
#[must_use]
pub fn classify(event: &a2a::StreamResponse) -> DeliveryKind {
    match event {
        a2a::StreamResponse::Message(_) => DeliveryKind::Interrupt,
        a2a::StreamResponse::Task(_)
        | a2a::StreamResponse::StatusUpdate(_)
        | a2a::StreamResponse::ArtifactUpdate(_) => DeliveryKind::Queue,
    }
}

/// The `inbox.accept` operation (security-relevant, fail-closed): the event goes in, its
/// reference comes out once it is stored.
#[derive(Debug)]
pub struct InboxAccept;

impl Operation for InboxAccept {
    const POINT: HookPoint = HookPoint::InboxAccept;
    type Input = InboxEvent;
    type Output = EventRef;
    fn identity(input: &InboxEvent) -> String {
        match &input.target {
            EventTarget::Task(t) | EventTarget::Context(t) => t.clone(),
        }
    }
}

/// The input of `inbox.deliver`: which event, how.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    /// The event.
    pub event: EventRef,
    /// Interrupt or queue, as the policy classified it.
    pub kind: DeliveryKind,
}

/// The output of `inbox.deliver`: the event was folded into the task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    /// The event.
    pub event: EventId,
    /// How it was delivered.
    pub kind: DeliveryKind,
}

/// The `inbox.deliver` operation (sherma `interrupt` with `DeliveryKind::Interrupt`).
#[derive(Debug)]
pub struct InboxDeliver;

impl Operation for InboxDeliver {
    const POINT: HookPoint = HookPoint::InboxDeliver;
    type Input = Delivery;
    type Output = Applied;
    fn identity(input: &Delivery) -> String {
        input.event.id.clone()
    }
}

/// The durable inbox (R20): one ingestion path for every event, A2A requests and
/// `/inbox` posts alike.
pub struct Inbox {
    store: Arc<dyn PersistenceStore>,
    durable: Arc<dyn DurableExecution>,
    agent: Arc<LocalAgent>,
    dispatcher: Arc<HookDispatcher>,
}

impl core::fmt::Debug for Inbox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Inbox").finish_non_exhaustive()
    }
}

impl Inbox {
    /// The inbox over the agent's store, signalling `durable`.
    #[must_use]
    pub fn new(
        agent: Arc<LocalAgent>,
        durable: Arc<dyn DurableExecution>,
        dispatcher: Arc<HookDispatcher>,
    ) -> Self {
        Self {
            store: Arc::clone(agent.store()),
            durable,
            agent,
            dispatcher,
        }
    }

    /// Validates `event` against its target (R20.6): the task must exist and be open,
    /// a message's context must match, and only a message may create a task. Returns the
    /// task the event lands on, when one exists already.
    ///
    /// # Errors
    ///
    /// [`InboxError`].
    pub async fn validate(
        &self,
        event: &InboxEvent,
    ) -> Result<Option<crate::harness::core_loop::task::TaskRecord>, InboxError> {
        let message = match &event.kind {
            a2a::StreamResponse::Message(m) => Some(m),
            _ => None,
        };
        let task_id = match &event.target {
            EventTarget::Task(t) => Some(t.clone()),
            EventTarget::Context(_) => message.and_then(|m| m.task_id.clone()),
        };
        let Some(task_id) = task_id else {
            return match message {
                Some(_) => Ok(None), // a message with no task creates one, as A2A defines
                None => Err(InboxError::Invalid("an update must name a task".into())),
            };
        };
        let Some(record) = self.store.get_task(&task_id).await? else {
            return Err(InboxError::NotFound(task_id));
        };
        if record.is_terminal() {
            return Err(InboxError::Terminal(task_id));
        }
        if let Some(got) = message.and_then(|m| m.context_id.clone())
            && !record.participants.is_empty()
            && got != record.context_id
        {
            return Err(InboxError::ContextMismatch {
                task: task_id,
                expected: record.context_id,
                got,
            });
        }
        if let EventTarget::Context(ctx) = &event.target
            && !record.participants.is_empty()
            && *ctx != record.context_id
        {
            return Err(InboxError::ContextMismatch {
                task: task_id,
                expected: record.context_id,
                got: ctx.clone(),
            });
        }
        Ok(Some(record))
    }

    /// `CancelTask` through durable execution (R22.6): the mode stops what it drives and
    /// the record ends `canceled`.
    ///
    /// # Errors
    ///
    /// [`InboxError::Durable`].
    pub async fn cancel(
        &self,
        task: &TaskId,
        reason: super::durable::CancelReason,
    ) -> Result<(), InboxError> {
        self.durable.cancel(task, reason).await?;
        Ok(())
    }

    /// Accepts `event`: validate, `inbox.accept` hooks, store as `Received`, apply or
    /// signal, mark `Signalled` (R20.3–5). Idempotent on the event id: a second accept of
    /// a stored event signals again and applies nothing twice.
    ///
    /// # Errors
    ///
    /// [`InboxError`].
    pub async fn accept(&self, mut event: InboxEvent) -> Result<EventRef, InboxError> {
        let existing = self.validate(&event).await?;
        // The event joins the task's trace; a new task's first event names it (R24.1).
        if event.traceparent.is_none() {
            event.traceparent = existing
                .as_ref()
                .and_then(|r| r.traceparent.clone())
                .or_else(|| Some(crate::app_service::o11y::trace::for_task(None)));
        }
        let mut cx = HookCx::new(
            Arc::new(crate::registry::Registry::new()),
            Arc::clone(self.agent.host()),
        );
        cx.task = existing
            .as_ref()
            .map(|r| r.id.clone())
            .or_else(|| match &event.target {
                EventTarget::Task(t) => Some(t.clone()),
                EventTarget::Context(_) => None,
            });
        cx.context_id = existing.as_ref().map(|r| r.context_id.clone());
        cx.traceparent.clone_from(&event.traceparent);
        let store = Arc::clone(&self.store);
        let event_for_hooks = event.clone();
        let reference = self
            .dispatcher
            .run::<InboxAccept, _, _>(&cx, event_for_hooks, move |e| async move {
                store
                    .put_inbox(&e)
                    .await
                    .map_err(|err| HarnessError::Internal(err.to_string()))
            })
            .await?;
        let task_id = match (&reference.target, &event.kind) {
            (EventTarget::Task(t), _) => t.clone(),
            (EventTarget::Context(_), a2a::StreamResponse::Message(m)) => {
                m.task_id.clone().unwrap_or_else(a2a::new_task_id)
            }
            (EventTarget::Context(_), _) => {
                return Err(InboxError::Invalid("an update must name a task".into()));
            }
        };
        // A duplicate (a retried signal, a redelivered webhook) is stored once and applied
        // once: the record's applied set and the pending list both know the id.
        let already = existing
            .as_ref()
            .is_some_and(|r| r.applied_events.contains(&event.id))
            || self
                .store
                .pending_inbox(&task_id)
                .await?
                .iter()
                .any(|e| e.id == event.id && e.status != InboxStatus::Received);
        // A message for a task the loop is not running right now is applied here, in the
        // same commit that marks it applied; the loop drains the rest at its boundary.
        if !already
            && let a2a::StreamResponse::Message(message) = &event.kind
            && !self.agent.mailbox().is_running(&task_id)
        {
            let context_id = existing
                .as_ref()
                .map(|r| r.context_id.clone())
                .or_else(|| message.context_id.clone())
                .unwrap_or_else(|| match &event.target {
                    EventTarget::Context(c) => c.clone(),
                    EventTarget::Task(_) => a2a::new_context_id(),
                });
            let request = AgentRequest {
                message: message.clone(),
                call: CallContext {
                    task_id: task_id.clone(),
                    context_id,
                    stored_task: None,
                    caller: event.sender.clone(),
                    service_params: std::collections::BTreeMap::new(),
                },
                metadata: None,
                extensions: event.extensions.clone(),
                inbox_event_id: Some(event.id.clone()),
                traceparent: event.traceparent.clone(),
            };
            self.agent.open_task(&request).await?;
        }
        self.durable
            .start_or_signal(&task_id, reference.clone())
            .await?;
        if let Some(stored) = self
            .store
            .pending_inbox(&task_id)
            .await?
            .into_iter()
            .find(|e| e.id == event.id)
            && stored.status == InboxStatus::Received
        {
            let at = self
                .agent
                .host()
                .clock()
                .map_or_else(|_| chrono::Utc::now(), |c| c.now());
            self.store
                .mark_inbox(&reference, InboxStatus::Signalled { at })
                .await?;
        }
        Ok(reference)
    }

    /// The events of `task` accepted and not yet applied, oldest first: what the loop
    /// drains at a boundary.
    ///
    /// # Errors
    ///
    /// The store.
    pub async fn drain(&self, task: &TaskId) -> Result<Vec<InboxEvent>, InboxError> {
        Ok(self.store.pending_inbox(task).await?)
    }

    /// The message a built-in surface sends instead of a refused follow-up on a terminal
    /// task (R21.6): the same text, no task id, the same context, the finished task in
    /// `referenceTaskIds`.
    #[must_use]
    pub fn follow_up_after_terminal(message: &a2a::Message, terminal: &a2a::Task) -> a2a::Message {
        let mut next = message.clone();
        next.message_id = a2a::new_message_id();
        next.task_id = None;
        next.context_id = Some(terminal.context_id.clone());
        let refs = next.reference_task_ids.get_or_insert_with(Vec::new);
        if !refs.contains(&terminal.id) {
            refs.push(terminal.id.clone());
        }
        // The new task keeps the mode the old one had, unless the message says otherwise.
        if let Some(mode) = terminal
            .metadata
            .as_ref()
            .and_then(|m| m.get(TASK_EXT))
            .and_then(|t| t.get("mode"))
            .cloned()
        {
            let meta = next
                .metadata
                .get_or_insert_with(std::collections::HashMap::new);
            let entry = meta
                .entry(TASK_EXT.to_owned())
                .or_insert_with(|| serde_json::json!({}));
            if entry.get("mode").is_none()
                && let Some(obj) = entry.as_object_mut()
            {
                obj.insert("mode".into(), mode);
            }
        }
        next
    }
}

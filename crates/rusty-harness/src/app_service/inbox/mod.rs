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

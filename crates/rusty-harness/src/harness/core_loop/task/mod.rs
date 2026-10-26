//! `TaskRecord`, participants, sub- and parent-task links and the state machine (R12,
//! R21).
//!
//! The record is the harness's task; on the wire it is projected to an [`a2a::Task`]
//! whose `metadata` carries, under the `task/v1` and `participants/v1` extension URIs,
//! the fields A2A does not model (R12.2). The A2A `state` only ever holds a standard
//! value; what A2A cannot say is a [`SubState`] on the status message's metadata
//! (R21.4). Transitions are one function, [`TaskRecord::transition`], which rejects an
//! illegal move with [`HarnessError::IllegalTransition`] and returns the status update
//! to emit (R12.5, R21.8).

use std::collections::{BTreeSet, HashMap};

use a2a::TaskState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::HarnessError;
use crate::harness::commands::ModelSelection;
use crate::harness::hooks::{HookPoint, Operation};

/// The A2A task id (a UUID v7 string).
pub type TaskId = a2a::TaskId;
/// The A2A context id.
pub type ContextId = String;
/// A participant's stable id.
pub type ParticipantId = String;
/// The id of a question asked of a participant (R18).
pub type AskId = String;

/// The extension URI prefix every rusty-harness A2A extension lives under (design.md
/// § A2A extensions).
pub const EXT_PREFIX: &str = "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/";
/// The `task/v1` extension URI: the metadata key for the record's extra fields.
pub const TASK_EXT: &str = "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/task/v1";
/// The `participants/v1` extension URI.
pub const PARTICIPANTS_EXT: &str =
    "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/participants/v1";
/// The `plan/v1` extension URI: the metadata key of a task's plan.
pub const PLAN_EXT: &str = "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/plan/v1";

/// Decodes an extension payload from A2A metadata. Metadata that crossed the A2A SDK is a
/// protobuf `Struct`, whose numbers are all doubles (`1` arrives as `1.0`), so whole-number
/// floats are read as integers first; every typed read of a `*/v1` payload goes through
/// here.
///
/// # Errors
///
/// The payload does not have `T`'s shape.
pub fn decode_extension<T: serde::de::DeserializeOwned>(
    value: &serde_json::Value,
) -> Result<T, serde_json::Error> {
    serde_json::from_value(integral(value.clone()))
}

/// `value` with every whole-number float made an integer.
fn integral(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Number(n) => match n.as_f64() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Some(f) if n.is_f64() && f.fract() == 0.0 && (0.0..9.0e15).contains(&f) => {
                Value::from(f as u64)
            }
            _ => Value::Number(n),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(integral).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, integral(v))).collect())
        }
        other => other,
    }
}

/// A participant's role on a task (R12.1).
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// Does the work.
    Assignee,
    /// Asked for it.
    Reporter,
    /// Follows it.
    Watcher,
    /// May change anything about it.
    Admin,
}

impl Role {
    /// The role's name as the string catalog and the `participants/v1` extension spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Assignee => "assignee",
            Self::Reporter => "reporter",
            Self::Watcher => "watcher",
            Self::Admin => "admin",
        }
    }

    /// The role spelled `name`, as a plugin's `commands.json` lists them.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Assignee, Self::Reporter, Self::Watcher, Self::Admin]
            .into_iter()
            .find(|r| r.name() == name)
    }
}

/// What kind of thing a participant is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ParticipantKind {
    /// A person.
    Human,
    /// An agent, by its registry identity.
    Agent(crate::registry::EntityId),
    /// A system (an event source, the harness itself).
    System,
}

/// A person, agent or system on a task, with its roles (R12.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Participant {
    /// Stable id (the authenticated caller's id for a person).
    pub id: ParticipantId,
    /// What to show.
    pub display: String,
    /// What it is.
    pub kind: ParticipantKind,
    /// Its roles.
    pub roles: BTreeSet<Role>,
}

impl Participant {
    /// A human participant with `roles`.
    #[must_use]
    pub fn human(
        id: impl Into<String>,
        display: impl Into<String>,
        roles: impl IntoIterator<Item = Role>,
    ) -> Self {
        Self {
            id: id.into(),
            display: display.into(),
            kind: ParticipantKind::Human,
            roles: roles.into_iter().collect(),
        }
    }

    /// Whether the participant holds `role`.
    #[must_use]
    pub fn has(&self, role: Role) -> bool {
        self.roles.contains(&role)
    }
}

/// Who is acting: the authenticated caller mapped to a participant (design.md § Security
/// design, boundary 1). Roles come from the task's participant list, never from text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caller {
    /// The caller's id.
    pub id: ParticipantId,
    /// The roles the caller holds on the task at hand; empty for a non-participant.
    pub roles: BTreeSet<Role>,
    /// Whether the caller is a person (commands are parsed from people only).
    pub human: bool,
    /// The channels the caller is a member of, for events visible to members only.
    #[serde(default)]
    pub channels: BTreeSet<String>,
}

impl Caller {
    /// A human caller with `roles`.
    #[must_use]
    pub fn human(id: impl Into<String>, roles: impl IntoIterator<Item = Role>) -> Self {
        Self {
            id: id.into(),
            roles: roles.into_iter().collect(),
            human: true,
            channels: BTreeSet::new(),
        }
    }

    /// A system or agent caller.
    #[must_use]
    pub fn system(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            roles: BTreeSet::new(),
            human: false,
            channels: BTreeSet::new(),
        }
    }

    /// The same caller, a member of `channels`.
    #[must_use]
    pub fn in_channels(mut self, channels: impl IntoIterator<Item = String>) -> Self {
        self.channels.extend(channels);
        self
    }

    /// Whether the caller may see a task: any role on it, or an admin.
    #[must_use]
    pub fn sees(&self, task: &TaskRecord) -> bool {
        task.participants.iter().any(|p| p.id == self.id)
    }

    /// Whether the caller may open a task at all, by any path (the A2A server's guard
    /// and the SDK alike): a task nobody has joined yet is open, any other only to its
    /// participants (abuse 2, design.md § Security design).
    #[must_use]
    pub fn may_open(&self, task: &TaskRecord) -> bool {
        task.participants.is_empty() || self.sees(task)
    }
}

/// Whether a task closes after one response or stays open across turns (R21.1).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum TaskMode {
    /// Completed once a response is delivered.
    #[default]
    OneShot,
    /// Moves to `input-required` after each response and waits for the next turn.
    LongRunning,
}

/// A condition A2A's state enum cannot express, carried on the status metadata (R21.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SubState {
    /// A long-running task delivered a response and waits for the next message.
    AwaitingNextTurn,
    /// Waiting for a participant's answer to `ask`.
    AwaitingParticipant {
        /// The question.
        ask: AskId,
    },
    /// Paused by a participant.
    Paused,
    /// Running with reduced capability; `reason` is a string-catalog key.
    Degraded {
        /// The catalog key of the reason.
        reason: String,
    },
    /// The context is being compacted.
    Compacting,
    /// Waiting on a sub-task.
    Delegated {
        /// The sub-task.
        task: TaskId,
    },
}

/// What happens to the non-terminal sub-tasks of a task that reached a terminal state
/// (R12.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubTaskPolicy {
    /// Cancel them.
    #[default]
    Cancel,
    /// Leave them running, unlinked from the parent.
    Detach,
    /// Keep the parent open until they finish.
    Wait,
}

/// A reference to another task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskRef {
    /// The task.
    pub task: TaskId,
    /// Its context, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<ContextId>,
}

/// The loop iteration counter.
pub type Iteration = u32;

/// One attempt of an activity: `(iteration, n)` (design.md § attempt protocol).
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub struct Attempt {
    /// The loop iteration.
    pub iteration: Iteration,
    /// The attempt within it, from 1.
    pub n: u32,
}

/// What a suspended tool call waits for (design.md § Tools, suspending tools).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum ResumeOn {
    /// A participant's answer.
    Ask {
        /// The question.
        ask: AskId,
    },
    /// A sub-task's terminal state.
    SubTask {
        /// The sub-task.
        task: TaskId,
    },
}

/// A tool call that suspended the loop and will be resumed with an answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCall {
    /// The model's call id.
    pub call_id: String,
    /// The tool.
    pub tool: String,
    /// What resumes it.
    pub resume_on: ResumeOn,
}

/// The record's optimistic-concurrency version: incremented by every commit.
pub type Version = u64;

/// What a creator supplies; everything else the harness fills (R12.1, R21.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDraft {
    /// The task id, chosen by the caller or minted by the harness.
    pub id: TaskId,
    /// The conversation it belongs to.
    pub context_id: ContextId,
    /// Short name.
    pub name: String,
    /// Optional type.
    #[serde(default)]
    pub r#type: Option<String>,
    /// What done looks like.
    pub goal: String,
    /// The longer description.
    #[serde(default)]
    pub description: String,
    /// How to tell it is done.
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    /// One-shot or long-running.
    #[serde(default)]
    pub mode: TaskMode,
    /// Who is on it.
    pub participants: Vec<Participant>,
    /// Parents, when the task is a sub-task.
    #[serde(default)]
    pub parent_tasks: Vec<TaskRef>,
    /// Finished tasks this one continues (R21.6).
    #[serde(default)]
    pub reference_task_ids: Vec<TaskId>,
    /// The model to use.
    pub model: ModelSelection,
}

/// The harness's task (R12.1); projected to [`a2a::Task`] on the wire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskRecord {
    /// The A2A task id.
    pub id: TaskId,
    /// The A2A context id.
    pub context_id: ContextId,
    /// Short name.
    pub name: String,
    /// Optional type.
    pub r#type: Option<String>,
    /// What done looks like.
    pub goal: String,
    /// The longer description.
    pub description: String,
    /// How to tell it is done.
    pub acceptance_criteria: Vec<String>,
    /// One-shot or long-running.
    pub mode: TaskMode,
    /// The A2A state; only standard values (R21.4).
    pub state: TaskState,
    /// What A2A cannot say about the state.
    pub sub_state: Option<SubState>,
    /// Who is on it.
    pub participants: Vec<Participant>,
    /// Tasks spawned for it (R12.3).
    pub sub_tasks: Vec<TaskRef>,
    /// Its parents; normally one entry.
    pub parent_tasks: Vec<TaskRef>,
    /// Its plan, once one exists.
    pub plan: Option<String>,
    /// The model selection for its next call.
    pub model: ModelSelection,
    /// When it was created.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
    /// The loop iteration reached.
    pub iteration: Iteration,
    /// Finished tasks it continues.
    pub reference_task_ids: Vec<TaskId>,
    /// The last status message (the delivered response).
    pub status_message: Option<a2a::Message>,
    /// Its artifacts.
    pub artifacts: Vec<a2a::Artifact>,
    /// How many history messages the projection carries.
    pub history_len: u64,
    /// Inbox events already applied, for exactly-once delivery (R20).
    pub applied_events: Vec<String>,
    /// Suspended tool calls.
    pub pending_calls: Vec<PendingCall>,
    /// The W3C `traceparent` every operation of the task joins (R24.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
    /// Optimistic-concurrency version.
    pub version: Version,
}

/// A move in the state machine (design.md § Task and lifecycle).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "transition")]
pub enum Transition {
    /// The workflow started: `submitted → working`.
    Start,
    /// A response was delivered: `working → completed` for a one-shot task,
    /// `working → input-required` (awaiting the next turn) for a long-running one.
    Delivered,
    /// A participant was asked: `working → input-required` (awaiting the participant).
    Ask {
        /// The question.
        ask: AskId,
    },
    /// A follow-up or an answer arrived: `input-required → working`.
    Resume,
    /// A participant completed the task: `working | input-required → completed`.
    Complete,
    /// `CancelTask`: `submitted | working | input-required → canceled`.
    Cancel,
    /// A non-retryable error or a limit: `submitted | working → failed`.
    Fail,
    /// A loop limit under the `input-required` policy: `working → input-required`,
    /// degraded with `reason` (a string-catalog key) so a person can decide (R11.4).
    Limit {
        /// The catalog key of the limit.
        reason: String,
    },
    /// A sub-state change while `working` (paused, degraded, compacting, delegated), or
    /// `None` to clear it.
    SubState(Option<SubState>),
}

impl Transition {
    /// The transition's name, for errors and traces.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Delivered => "delivered",
            Self::Ask { .. } => "ask",
            Self::Resume => "resume",
            Self::Complete => "complete",
            Self::Cancel => "cancel",
            Self::Fail => "fail",
            Self::Limit { .. } => "limit",
            Self::SubState(_) => "sub-state",
        }
    }
}

/// The record's extension payload (`task/v1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskMeta {
    /// Short name.
    pub name: String,
    /// Optional type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// What done looks like.
    pub goal: String,
    /// The longer description.
    pub description: String,
    /// How to tell it is done.
    pub acceptance_criteria: Vec<String>,
    /// One-shot or long-running.
    pub mode: TaskMode,
    /// Spawned tasks.
    pub sub_tasks: Vec<TaskRef>,
    /// Parents.
    pub parent_tasks: Vec<TaskRef>,
    /// Finished tasks this one continues.
    pub reference_task_ids: Vec<TaskId>,
    /// The sub-state, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_state: Option<SubState>,
}

/// The `participants/v1` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ParticipantsMeta {
    /// Everyone on the task.
    pub participants: Vec<Participant>,
}

impl TaskRecord {
    /// A new, `submitted` record from a draft at `now`.
    #[must_use]
    pub fn new(draft: TaskDraft, now: DateTime<Utc>) -> Self {
        Self {
            traceparent: None,
            id: draft.id,
            context_id: draft.context_id,
            name: draft.name,
            r#type: draft.r#type,
            goal: draft.goal,
            description: draft.description,
            acceptance_criteria: draft.acceptance_criteria,
            mode: draft.mode,
            state: TaskState::Submitted,
            sub_state: None,
            participants: draft.participants,
            sub_tasks: Vec::new(),
            parent_tasks: draft.parent_tasks,
            plan: None,
            model: draft.model,
            created_at: now,
            updated_at: now,
            iteration: 0,
            reference_task_ids: draft.reference_task_ids,
            status_message: None,
            artifacts: Vec::new(),
            history_len: 0,
            applied_events: Vec::new(),
            pending_calls: Vec::new(),
            version: 0,
        }
    }

    /// Whether no further change is allowed.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }

    /// The target state of `transition` from the current state, or `None` when the move
    /// is illegal. The table is the state diagram of design.md § Task and lifecycle.
    #[must_use]
    pub fn target(&self, transition: &Transition) -> Option<(TaskState, Option<SubState>)> {
        use TaskState::{Canceled, Completed, Failed, InputRequired, Submitted, Working};
        let working_or_waiting = matches!(self.state, Working | InputRequired);
        match (transition, self.state.clone()) {
            (Transition::Start, Submitted) | (Transition::Resume, InputRequired) => {
                Some((Working, None))
            }
            (Transition::Delivered, Working) => Some(match self.mode {
                TaskMode::OneShot => (Completed, None),
                TaskMode::LongRunning => (InputRequired, Some(SubState::AwaitingNextTurn)),
            }),
            (Transition::Ask { ask }, Working) => Some((
                InputRequired,
                Some(SubState::AwaitingParticipant { ask: ask.clone() }),
            )),
            (Transition::Complete, _) if working_or_waiting => Some((Completed, None)),
            (Transition::Cancel, _) if working_or_waiting || self.state == Submitted => {
                Some((Canceled, None))
            }
            (Transition::Fail, Working | Submitted) => Some((Failed, None)),
            (Transition::Limit { reason }, Working) => Some((
                InputRequired,
                Some(SubState::Degraded {
                    reason: reason.clone(),
                }),
            )),
            (Transition::SubState(sub), Working) => Some((Working, sub.clone())),
            _ => None,
        }
    }

    /// Applies `transition` at `now`, returning the A2A status update to emit (R12.5,
    /// R21.8). The record's `version` is not touched: the store's commit does that.
    ///
    /// # Errors
    ///
    /// [`HarnessError::IllegalTransition`] when the move is not in the table; the record
    /// is unchanged.
    pub fn transition(
        &mut self,
        transition: &Transition,
        now: DateTime<Utc>,
    ) -> Result<a2a::TaskStatusUpdateEvent, HarnessError> {
        let Some((state, sub_state)) = self.target(transition) else {
            return Err(HarnessError::IllegalTransition {
                from: self.state.clone(),
                to: transition.name(),
            });
        };
        let terminal = state.is_terminal();
        self.state = state;
        self.sub_state = sub_state;
        self.updated_at = now;
        if terminal {
            self.pending_calls.clear();
        }
        Ok(self.status_update())
    }

    /// The status (state, last message, timestamp) as A2A carries it, with the sub-state
    /// on the message's metadata when there is one (R21.3–4).
    #[must_use]
    pub fn status(&self) -> a2a::TaskStatus {
        let mut message = self.status_message.clone();
        if let Some(sub) = &self.sub_state {
            let msg =
                message.get_or_insert_with(|| a2a::Message::new(a2a::Role::Agent, Vec::new()));
            msg.task_id = Some(self.id.clone());
            msg.context_id = Some(self.context_id.clone());
            let meta = msg.metadata.get_or_insert_with(HashMap::new);
            meta.insert(TASK_EXT.to_owned(), serde_json::json!({ "sub_state": sub }));
        }
        a2a::TaskStatus {
            state: self.state.clone(),
            message,
            timestamp: Some(self.updated_at),
        }
    }

    /// The `TaskStatusUpdateEvent` for the current status.
    #[must_use]
    pub fn status_update(&self) -> a2a::TaskStatusUpdateEvent {
        a2a::TaskStatusUpdateEvent {
            task_id: self.id.clone(),
            context_id: self.context_id.clone(),
            status: self.status(),
            metadata: Some(self.metadata()),
        }
    }

    /// The extension metadata (`task/v1`, `participants/v1`).
    #[must_use]
    pub fn metadata(&self) -> HashMap<String, serde_json::Value> {
        let meta = TaskMeta {
            name: self.name.clone(),
            r#type: self.r#type.clone(),
            goal: self.goal.clone(),
            description: self.description.clone(),
            acceptance_criteria: self.acceptance_criteria.clone(),
            mode: self.mode,
            sub_tasks: self.sub_tasks.clone(),
            parent_tasks: self.parent_tasks.clone(),
            reference_task_ids: self.reference_task_ids.clone(),
            sub_state: self.sub_state.clone(),
        };
        let participants = ParticipantsMeta {
            participants: self.participants.clone(),
        };
        HashMap::from([
            (
                TASK_EXT.to_owned(),
                serde_json::to_value(meta).unwrap_or(serde_json::Value::Null),
            ),
            (
                PARTICIPANTS_EXT.to_owned(),
                serde_json::to_value(participants).unwrap_or(serde_json::Value::Null),
            ),
        ])
    }

    /// The projection to the wire (R12.2): `history` is the task's message log, which
    /// the store supplies bounded by `history_len`.
    #[must_use]
    pub fn to_a2a(&self, history: Vec<a2a::Message>) -> a2a::Task {
        a2a::Task {
            id: self.id.clone(),
            context_id: self.context_id.clone(),
            status: self.status(),
            artifacts: (!self.artifacts.is_empty()).then(|| self.artifacts.clone()),
            history: (!history.is_empty()).then_some(history),
            metadata: Some(self.metadata()),
        }
    }

    /// Folds an artifact update into the record: appended to the artifact with the same
    /// id when `append`, replaced or added otherwise.
    pub fn apply_artifact(&mut self, update: &a2a::TaskArtifactUpdateEvent) {
        let existing = self
            .artifacts
            .iter_mut()
            .find(|a| a.artifact_id == update.artifact.artifact_id);
        match (existing, update.append.unwrap_or(false)) {
            (Some(a), true) => a.parts.extend(update.artifact.parts.iter().cloned()),
            (Some(a), false) => *a = update.artifact.clone(),
            (None, _) => self.artifacts.push(update.artifact.clone()),
        }
    }

    /// Adds a sub-task reference (R12.3).
    pub fn link_sub_task(&mut self, task: TaskRef) {
        if !self.sub_tasks.iter().any(|t| t.task == task.task) {
            self.sub_tasks.push(task);
        }
    }
}

/// The six rusty-harness A2A extensions (design.md § A2A extensions, Q12): their URIs and
/// the JSON schema of each payload, generated from the Rust types and checked in under
/// `specs/a2a-ext/<name>.v1.schema.json` (contract test `contract_a2a_extension_schemas`).
pub mod ext {
    use schemars::{Schema, schema_for};

    /// One extension: its short name, URI and payload schema.
    #[derive(Clone, Debug)]
    pub struct Extension {
        /// `task`, `participants`, `plan`, `channels`, `commands`, `voice`.
        pub name: &'static str,
        /// The URI advertised in the card and used as the metadata key.
        pub uri: String,
        /// The payload schema.
        pub schema: Schema,
    }

    fn uri(name: &str) -> String {
        format!("{}{name}/v1", super::EXT_PREFIX)
    }

    /// Every extension, in the card's order.
    #[must_use]
    pub fn all() -> Vec<Extension> {
        vec![
            Extension {
                name: "task",
                uri: uri("task"),
                schema: schema_for!(super::TaskMeta),
            },
            Extension {
                name: "participants",
                uri: uri("participants"),
                schema: schema_for!(super::ParticipantsMeta),
            },
            Extension {
                name: "plan",
                uri: uri("plan"),
                schema: schema_for!(crate::harness::core_loop::plan::Plan),
            },
            Extension {
                name: "channels",
                uri: uri("channels"),
                schema: schema_for!(crate::harness::channels::ChannelsMeta),
            },
            Extension {
                name: "commands",
                uri: uri("commands"),
                schema: schema_for!(crate::harness::commands::CommandsMeta),
            },
            Extension {
                name: "voice",
                uri: uri("voice"),
                schema: schema_for!(crate::harness::models::speech::VoiceMeta),
            },
        ]
    }

    /// The URIs, for the card's `capabilities.extensions`.
    #[must_use]
    pub fn uris() -> Vec<String> {
        all().into_iter().map(|e| e.uri).collect()
    }
}

/// The `task.create` operation: a draft goes in, the record comes out.
#[derive(Debug)]
pub struct TaskCreate;

impl Operation for TaskCreate {
    const POINT: HookPoint = HookPoint::TaskCreate;
    type Input = TaskDraft;
    type Output = TaskRecord;
    fn identity(input: &TaskDraft) -> String {
        input.id.clone()
    }
}

/// The input of `task.state_change`: the record and the move.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateChange {
    /// The record before the move.
    pub task: TaskRecord,
    /// The move.
    pub transition: Transition,
    /// When.
    pub at: DateTime<Utc>,
}

/// The output of `task.state_change`: the record after the move and the event to emit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateChanged {
    /// The record after the move.
    pub task: TaskRecord,
    /// The A2A status update (R12.5).
    pub event: a2a::TaskStatusUpdateEvent,
}

/// The `task.state_change` operation (R12.5): `pre` hooks may veto a move.
#[derive(Debug)]
pub struct TaskStateChange;

impl Operation for TaskStateChange {
    const POINT: HookPoint = HookPoint::TaskStateChange;
    type Input = StateChange;
    type Output = StateChanged;
    fn identity(input: &StateChange) -> String {
        format!("{}:{}", input.task.id, input.transition.name())
    }
}

/// Runs `task.state_change` hooks around [`TaskRecord::transition`].
///
/// # Errors
///
/// An illegal move, a veto or a hook failure.
pub async fn transition_with_hooks(
    dispatcher: &crate::harness::hooks::HookDispatcher,
    cx: &crate::harness::hooks::HookCx,
    change: StateChange,
) -> Result<StateChanged, HarnessError> {
    dispatcher
        .run::<TaskStateChange, _, _>(cx, change, |change| async move {
            let mut task = change.task;
            let event = task.transition(&change.transition, change.at)?;
            Ok(StateChanged { task, event })
        })
        .await
}

#[cfg(test)]
pub(crate) mod tests {

    /// A payload whose numbers crossed the wire as doubles decodes as integers; a
    /// fractional number is still refused where an integer is wanted.
    #[test]
    fn an_extension_payload_with_double_numbers_decodes() {
        #[derive(serde::Deserialize)]
        struct Versioned {
            version: u64,
        }
        let plan = serde_json::json!({ "version": 3.0 });
        let decoded: Versioned = super::decode_extension(&plan).expect("decodes");
        assert_eq!(decoded.version, 3);
        assert!(
            super::decode_extension::<Versioned>(&serde_json::json!({ "version": 3.5 })).is_err()
        );
    }

    use super::{
        Caller, Participant, Role, StateChange, SubState, TASK_EXT, TaskDraft, TaskMode,
        TaskRecord, Transition, transition_with_hooks,
    };
    use crate::HarnessError;
    use crate::harness::commands::ModelSelection;
    use crate::harness::hooks::{HookCx, HookDispatcher};
    use crate::host::Host;
    use crate::registry::Registry;
    use a2a::TaskState;
    use chrono::{TimeZone, Utc};
    use std::sync::Arc;

    pub(crate) fn draft(id: &str, mode: TaskMode) -> TaskDraft {
        TaskDraft {
            id: id.into(),
            context_id: "ctx-1".into(),
            name: "demo".into(),
            r#type: None,
            goal: "answer".into(),
            description: String::new(),
            acceptance_criteria: vec!["it answers".into()],
            mode,
            participants: vec![Participant::human(
                "u1",
                "sam",
                [Role::Reporter, Role::Admin],
            )],
            parent_tasks: Vec::new(),
            reference_task_ids: Vec::new(),
            model: ModelSelection {
                model: "default".into(),
                effort: None,
            },
        }
    }

    fn at(n: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, n).unwrap()
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn every_legal_transition_of_the_diagram_is_accepted() {
        let legal: &[(TaskState, TaskMode, Transition, TaskState, Option<SubState>)] = &[
            (
                TaskState::Submitted,
                TaskMode::OneShot,
                Transition::Start,
                TaskState::Working,
                None,
            ),
            (
                TaskState::Working,
                TaskMode::OneShot,
                Transition::Delivered,
                TaskState::Completed,
                None,
            ),
            (
                TaskState::Working,
                TaskMode::LongRunning,
                Transition::Delivered,
                TaskState::InputRequired,
                Some(SubState::AwaitingNextTurn),
            ),
            (
                TaskState::Working,
                TaskMode::OneShot,
                Transition::Ask { ask: "q1".into() },
                TaskState::InputRequired,
                Some(SubState::AwaitingParticipant { ask: "q1".into() }),
            ),
            (
                TaskState::InputRequired,
                TaskMode::LongRunning,
                Transition::Resume,
                TaskState::Working,
                None,
            ),
            (
                TaskState::Working,
                TaskMode::LongRunning,
                Transition::Complete,
                TaskState::Completed,
                None,
            ),
            (
                TaskState::InputRequired,
                TaskMode::LongRunning,
                Transition::Complete,
                TaskState::Completed,
                None,
            ),
            (
                TaskState::Working,
                TaskMode::OneShot,
                Transition::Cancel,
                TaskState::Canceled,
                None,
            ),
            (
                TaskState::InputRequired,
                TaskMode::OneShot,
                Transition::Cancel,
                TaskState::Canceled,
                None,
            ),
            (
                TaskState::Submitted,
                TaskMode::OneShot,
                Transition::Cancel,
                TaskState::Canceled,
                None,
            ),
            (
                TaskState::Working,
                TaskMode::OneShot,
                Transition::Fail,
                TaskState::Failed,
                None,
            ),
            (
                TaskState::Submitted,
                TaskMode::OneShot,
                Transition::Fail,
                TaskState::Failed,
                None,
            ),
            (
                TaskState::Working,
                TaskMode::OneShot,
                Transition::SubState(Some(SubState::Paused)),
                TaskState::Working,
                Some(SubState::Paused),
            ),
            (
                TaskState::Working,
                TaskMode::OneShot,
                Transition::Limit {
                    reason: "loop.limit".into(),
                },
                TaskState::InputRequired,
                Some(SubState::Degraded {
                    reason: "loop.limit".into(),
                }),
            ),
        ];
        for (from, mode, transition, to, sub) in legal {
            let mut task = TaskRecord::new(draft("t", *mode), at(0));
            task.state = from.clone();
            let event = task
                .transition(transition, at(1))
                .unwrap_or_else(|e| panic!("{from:?} --{transition:?}--> {e}"));
            assert_eq!(task.state, *to, "{from:?} --{transition:?}");
            assert_eq!(&task.sub_state, sub);
            assert_eq!(event.status.state, *to);
            assert_eq!(task.updated_at, at(1));
            if let Some(sub) = sub {
                let meta = event
                    .status
                    .message
                    .as_ref()
                    .and_then(|m| m.metadata.as_ref())
                    .and_then(|m| m.get(TASK_EXT))
                    .expect("sub-state on the status message");
                assert_eq!(meta["sub_state"], serde_json::to_value(sub).unwrap());
            }
        }
    }

    #[test]
    fn every_illegal_transition_is_a_typed_error_and_changes_nothing() {
        let illegal: &[(TaskState, Transition)] = &[
            (TaskState::Submitted, Transition::Delivered),
            (TaskState::Submitted, Transition::Resume),
            (TaskState::Submitted, Transition::Complete),
            (TaskState::Working, Transition::Start),
            (TaskState::Working, Transition::Resume),
            (TaskState::InputRequired, Transition::Start),
            (TaskState::InputRequired, Transition::Delivered),
            (TaskState::InputRequired, Transition::Fail),
            (
                TaskState::InputRequired,
                Transition::Limit { reason: "x".into() },
            ),
            (
                TaskState::InputRequired,
                Transition::SubState(Some(SubState::Paused)),
            ),
            (TaskState::Completed, Transition::Resume),
            (TaskState::Completed, Transition::Cancel),
            (TaskState::Completed, Transition::Complete),
            (TaskState::Canceled, Transition::Start),
            (TaskState::Failed, Transition::Resume),
        ];
        for (from, transition) in illegal {
            let mut task = TaskRecord::new(draft("t", TaskMode::LongRunning), at(0));
            task.state = from.clone();
            let before = task.clone();
            let err = task
                .transition(transition, at(1))
                .expect_err(&format!("{from:?} --{transition:?}"));
            assert!(
                matches!(&err, HarnessError::IllegalTransition { from: f, to } if f == from && *to == transition.name()),
                "{err}"
            );
            assert_eq!(task, before, "unchanged");
        }
        // No value is ever added to TaskState: the sub-state rides on metadata (R21.4).
        let mut task = TaskRecord::new(draft("t", TaskMode::LongRunning), at(0));
        task.transition(&Transition::Start, at(1)).unwrap();
        task.transition(&Transition::Delivered, at(2)).unwrap();
        assert_eq!(task.state, TaskState::InputRequired);
        let wire = task.to_a2a(Vec::new());
        assert_eq!(wire.status.state, TaskState::InputRequired);
        assert_eq!(
            wire.metadata.as_ref().unwrap()[TASK_EXT]["mode"],
            "long-running"
        );
        assert_eq!(
            wire.metadata.as_ref().unwrap()[super::PARTICIPANTS_EXT]["participants"][0]["roles"][0],
            "reporter"
        );
        assert!(Caller::human("u1", []).sees(&task));
        assert!(!Caller::human("stranger", []).sees(&task));
    }

    #[tokio::test]
    async fn a_state_change_runs_its_hooks_and_a_terminal_move_drops_pending_calls() {
        let d = HookDispatcher::new();
        let cx = HookCx::new(Arc::new(Registry::new()), Arc::new(Host::testing()));
        let mut task = TaskRecord::new(draft("t", TaskMode::OneShot), at(0));
        task.state = TaskState::Working;
        task.pending_calls.push(super::PendingCall {
            call_id: "c1".into(),
            tool: "ask".into(),
            resume_on: super::ResumeOn::Ask { ask: "q".into() },
        });
        let changed = transition_with_hooks(
            &d,
            &cx,
            StateChange {
                task,
                transition: Transition::Cancel,
                at: at(3),
            },
        )
        .await
        .unwrap();
        assert_eq!(changed.task.state, TaskState::Canceled);
        assert_eq!(changed.task.pending_calls.len(), 0);
        assert_eq!(changed.event.task_id, "t");
    }
}

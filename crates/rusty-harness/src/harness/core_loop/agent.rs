//! The `Agent` trait (sherma's base agent in A2A types) and `LocalAgent`, the loop (R11).
//!
//! [`Agent`] is sherma's base agent translated to the official A2A Rust types:
//! `card()`, `schemas()`, `send_message()` as a stream of `StreamResponse`, and
//! `cancel_task()` (design.md § Agent and the core loop, the mapping table). Local
//! agents, remote agents and loopback sub-agents all implement it.
//!
//! [`LocalAgent`] runs the canonical loop: build the context, call the model, parse the
//! tool calls, execute them, repeat; each step is a hook point (`context.build`,
//! `model.call`, `response.parse`, `tool.execute`), the whole loop is `task.run`. Events
//! and signals that arrive while a step runs are observed at the loop boundary (R11.5);
//! limits stop the loop with a typed [`LoopOutcome`] (R11.4); a response with no tool
//! calls is delivered and the task's mode decides between `completed` and
//! `input-required` (R11.3, R21.1, R21.3).

use crate::runtime::Instant;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::HarnessError;
use crate::app_service::durable::CancelReason;
use crate::app_service::inbox::InboxEvent;
use crate::assets::{Prompt, PromptRegistry};
use crate::config::Config;
use crate::harness::commands::ModelRouter;
use crate::harness::core_loop::compaction::{CompactionRequest, Compactor};
use crate::harness::core_loop::context_window::{
    ContextBuilder, ContextInputs, ContextTemplate, HistoryEntry, TokenRatio, Turn, request_chars,
};
use crate::harness::core_loop::persistence::{
    BlobWrite, PersistenceStore, StoreError, StoredEvent, TaskCommit, TaskQuery, Visibility,
};
use crate::harness::core_loop::plan::Plan;
use crate::harness::core_loop::state::AgentState;
use crate::harness::core_loop::task::{
    Attempt, Caller, ContextId, Participant, ParticipantId, ParticipantKind, PendingCall, ResumeOn,
    Role, StateChange, SubTaskPolicy, TASK_EXT, TaskDraft, TaskId, TaskMode, TaskRecord,
    Transition, transition_with_hooks,
};
use crate::harness::hooks::{HookCx, HookDispatcher, HookPoint, Operation};
use crate::harness::models::llm::{Modality, ModelError, ModelRequest, ModelTurn, collect};
use crate::harness::skills::Skill;
use crate::harness::system_prompt::SystemPrompt;
use crate::harness::tools::{CancelToken, Executed, OversizePolicy, ToolCall, ToolExecutor};
use crate::harness_async_trait;
use crate::host::Host;
use crate::registry::{Entity, EntityId, EntityKind, Registry};
use crate::runtime::{HarnessStream, boxed_stream};

/// What the service layer knows about one call (sherma's `context`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CallContext {
    /// The task the message addresses; a new id creates a task.
    pub task_id: TaskId,
    /// The conversation.
    pub context_id: ContextId,
    /// The stored task, when the service layer already loaded it.
    #[serde(default)]
    pub stored_task: Option<a2a::Task>,
    /// The authenticated caller (boundary 1).
    pub caller: Caller,
    /// Transport parameters the service layer passes through.
    #[serde(default)]
    pub service_params: BTreeMap<String, String>,
}

/// One `send_message` (sherma's `message, context, request_metadata, extensions`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentRequest {
    /// The message.
    pub message: a2a::Message,
    /// The call.
    pub call: CallContext,
    /// Request metadata.
    #[serde(default)]
    pub metadata: Option<HashMap<String, Value>>,
    /// Extension URIs the caller activated.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// The inbox event this message arrived as, marked applied by the commit that
    /// applies it (R20.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox_event_id: Option<String>,
    /// The W3C `traceparent` the request arrived with; a new task adopts it (R24.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
}

/// The agent's input and output schemas (sherma's `input_schema` / `output_schema`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentSchemas {
    /// JSON Schema of what it accepts, when declared.
    pub input: Option<Value>,
    /// JSON Schema of what it produces, when declared.
    pub output: Option<Value>,
}

/// What an agent call can fail with.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// A harness failure (a veto, a hook failure, an illegal move).
    #[error(transparent)]
    Harness(#[from] HarnessError),
    /// The store.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The protocol forbids it: a follow-up on a terminal task (R21.6), a task of
    /// another context.
    #[error("unsupported operation: {0}")]
    Unsupported(String),
    /// The model.
    #[error("model: {0}")]
    Model(String),
    /// The message carries a modality the selected model does not accept (R28.1):
    /// `ContentTypeNotSupported` on the wire, naming the modality.
    #[error("the model {model} does not accept {modality:?} input")]
    UnsupportedModality {
        /// What was sent.
        modality: Modality,
        /// The model alias.
        model: String,
    },
    /// The task is not visible to the caller (a non-member of a channel, a
    /// non-participant): `TaskNotFound` on the wire (abuse 2).
    #[error("task {0}: not found")]
    NotFound(TaskId),
}

impl AgentError {
    /// The A2A error for the wire (R19.4).
    #[must_use]
    pub fn to_a2a(&self) -> a2a::A2AError {
        match self {
            Self::Harness(e) => e.to_a2a(),
            Self::Store(e) => a2a::A2AError::internal(e.to_string()),
            Self::Unsupported(m) => a2a::A2AError::unsupported_operation(m.clone()),
            Self::Model(m) => a2a::A2AError::internal(m.clone()),
            Self::UnsupportedModality { .. } => {
                let mut e = a2a::A2AError::content_type_not_supported();
                e.message = self.to_string();
                e
            }
            Self::NotFound(t) => a2a::A2AError::task_not_found(t),
        }
    }
}

harness_async_trait! {
    /// sherma's base agent, in A2A types (R11.1).
    pub trait Agent: Entity {
        /// `agent_card` / `get_card()`.
        fn card(&self) -> &a2a::AgentCard;
        /// `input_schema` / `output_schema`.
        fn schemas(&self) -> AgentSchemas;
        /// `send_message(message, context, request_metadata, extensions)`: the events the
        /// call produced, as A2A streams them.
        fn send_message(&self, request: AgentRequest) -> HarnessStream<'_, Result<a2a::StreamResponse, AgentError>>;
        /// `cancel_task(task_id)`: the task after the stop path.
        async fn cancel_task(&self, task_id: &TaskId, call: CallContext) -> Result<a2a::Task, AgentError>;
        /// `list_tasks(context_id)`: the tasks of one context the caller may see, for the
        /// outbound reconciliation (R17.4, design.md § Durable execution).
        async fn list_tasks(&self, context_id: &ContextId, call: CallContext) -> Result<Vec<a2a::Task>, AgentError>;
    }
}

/// The loop's budgets (R11.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopLimits {
    /// Model calls per task.
    pub max_iterations: u32,
    /// Prompt plus completion tokens per task, when set.
    pub max_total_tokens: Option<u64>,
    /// Wall clock per task, when set.
    pub wall_clock: Option<std::time::Duration>,
}

impl Default for LoopLimits {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            max_total_tokens: None,
            wall_clock: None,
        }
    }
}

/// Which limit stopped the loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Limit {
    /// `max_iterations`.
    MaxIterations,
    /// `max_total_tokens`.
    MaxTokens,
    /// `wall_clock`.
    WallClock,
}

impl Limit {
    /// The string-catalog key the degraded sub-state carries.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::MaxIterations => "loop.limit.iterations",
            Self::MaxTokens => "loop.limit.tokens",
            Self::WallClock => "loop.limit.wall_clock",
        }
    }
}

/// What a limit does to the task (`[loop] on_limit`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OnLimit {
    /// `input-required`, degraded, for a person to decide.
    #[default]
    InputRequired,
    /// `failed`.
    Fail,
}

/// How a run of the loop ended (R11.4, design.md § Agent and the core loop).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "outcome")]
pub enum LoopOutcome {
    /// The task is `completed`.
    Completed,
    /// A long-running task delivered and waits for the next turn.
    AwaitingTurn,
    /// A suspending tool waits for a participant.
    AwaitingParticipant {
        /// The question.
        ask: String,
    },
    /// The task is `canceled`.
    Canceled,
    /// A limit stopped the loop.
    LimitReached {
        /// Which.
        limit: Limit,
    },
    /// The task is `failed`.
    Failed(String),
}

/// What `build_context` found (phase 1 of an iteration).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "begun")]
pub enum Begun {
    /// The request is built and persisted; call the model.
    Context {
        /// The blob holding the request (`ctx/<task>/<n>`).
        ctx_key: String,
        /// The request's size, for the token ratio.
        chars: usize,
        /// The model alias the router selected.
        model: String,
    },
    /// The boundary ended the run (a signal, a limit, a waiting task).
    Stopped {
        /// How.
        outcome: LoopOutcome,
    },
}

/// What `call_model` produced (phase 2 of an iteration).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "model")]
pub enum ModelPhase {
    /// The turn is persisted; run its calls, then finish the iteration.
    Turn {
        /// The blob holding the turn (`turn/<task>/<n>`).
        turn_key: String,
        /// The ids of the tool calls the parse kept, in issue order.
        calls: Vec<String>,
        /// Why the model stopped.
        stop: Option<crate::harness::models::llm::StopReason>,
        /// Token usage, when the provider reported it.
        usage: Option<crate::harness::models::llm::Usage>,
    },
    /// The model's failure ended the task (a veto, a non-retryable provider error).
    Stopped {
        /// How.
        outcome: LoopOutcome,
    },
}

/// What one iteration decided.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "iteration")]
pub enum IterationOutcome {
    /// Tools ran; call the model again.
    Continue,
    /// A response was delivered; the task's mode decided the state.
    Delivered,
    /// A tool suspended the loop.
    Suspended {
        /// The question.
        ask: String,
    },
    /// A signal ended the task.
    Stopped {
        /// How.
        outcome: LoopOutcome,
    },
}

/// The `task.run` operation: the whole loop (sherma `graph_invoke`).
#[derive(Debug)]
pub struct TaskRun;

impl Operation for TaskRun {
    const POINT: HookPoint = HookPoint::TaskRun;
    type Input = TaskId;
    type Output = LoopOutcome;
    fn identity(input: &TaskId) -> String {
        input.clone()
    }
}

/// The `model.call` operation (sherma `model_call`).
#[derive(Debug)]
pub struct ModelCall;

impl Operation for ModelCall {
    const POINT: HookPoint = HookPoint::ModelCall;
    type Input = ModelRequest;
    type Output = ModelTurn;
    fn attributes(input: &ModelRequest, output: &ModelTurn) -> Vec<(&'static str, String)> {
        vec![
            ("messages", input.messages.len().to_string()),
            ("tool_calls", output.tool_calls.len().to_string()),
            (
                "tokens.in",
                output
                    .usage
                    .as_ref()
                    .map_or_else(String::new, |u| u.input_tokens.to_string()),
            ),
        ]
    }
}

/// The `response.parse` operation: the model's turn in, its tool calls out.
#[derive(Debug)]
pub struct ResponseParse;

impl Operation for ResponseParse {
    const POINT: HookPoint = HookPoint::ResponseParse;
    type Input = ModelTurn;
    type Output = Vec<ToolCall>;
}

/// What `response.deliver` delivers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    /// The task.
    pub task: TaskId,
    /// The response.
    pub message: a2a::Message,
}

/// The `response.deliver` operation (R11.3).
#[derive(Debug)]
pub struct ResponseDeliver;

impl Operation for ResponseDeliver {
    const POINT: HookPoint = HookPoint::ResponseDeliver;
    type Input = Delivery;
    type Output = Delivery;
    fn identity(input: &Delivery) -> String {
        input.task.clone()
    }
}

/// A stop signal for a running task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signal {
    /// `CancelTask`.
    Cancel(CancelReason),
    /// The `complete` transition.
    Complete {
        /// Who.
        by: ParticipantId,
    },
}

#[derive(Default)]
struct MailboxInner {
    queues: BTreeMap<TaskId, VecDeque<InboxEvent>>,
    signals: BTreeMap<TaskId, Signal>,
    tokens: BTreeMap<TaskId, CancelToken>,
    running: BTreeSet<TaskId>,
}

/// The in-memory inbox of a running loop (R11.5, R20.4): events and stop signals are
/// queued here and observed at the next loop boundary. The durable modes' inbox and
/// signals land here too, in process.
#[derive(Default)]
pub struct Mailbox {
    inner: Mutex<MailboxInner>,
}

impl core::fmt::Debug for Mailbox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("Mailbox")
            .field("running", &inner.running.len())
            .finish_non_exhaustive()
    }
}

impl Mailbox {
    fn lock(&self) -> std::sync::MutexGuard<'_, MailboxInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues an event for the task's next loop boundary.
    pub fn deliver(&self, task: &TaskId, event: InboxEvent) {
        self.lock()
            .queues
            .entry(task.clone())
            .or_default()
            .push_back(event);
    }

    /// The queued events, oldest first.
    #[must_use]
    pub fn drain(&self, task: &TaskId) -> Vec<InboxEvent> {
        self.lock()
            .queues
            .remove(task)
            .map(Vec::from)
            .unwrap_or_default()
    }

    /// Sends a stop signal and cancels the task's in-flight activities.
    pub fn signal(&self, task: &TaskId, signal: Signal) {
        let mut inner = self.lock();
        if let Some(token) = inner.tokens.get(task) {
            token.cancel();
        }
        inner.signals.insert(task.clone(), signal);
    }

    /// Takes the pending stop signal, if any.
    #[must_use]
    pub fn take_signal(&self, task: &TaskId) -> Option<Signal> {
        self.lock().signals.remove(task)
    }

    /// Whether a loop is running the task now.
    #[must_use]
    pub fn is_running(&self, task: &TaskId) -> bool {
        self.lock().running.contains(task)
    }

    /// The task's cancel token, created on first use.
    #[must_use]
    pub fn token(&self, task: &TaskId) -> CancelToken {
        self.lock().tokens.entry(task.clone()).or_default().clone()
    }

    fn start(&self, task: &TaskId) -> bool {
        let mut inner = self.lock();
        inner.tokens.insert(task.clone(), CancelToken::new());
        inner.running.insert(task.clone())
    }

    fn finish(&self, task: &TaskId) {
        let mut inner = self.lock();
        inner.running.remove(task);
        inner.tokens.remove(task);
    }
}

/// The local agent: the loop over the registry, the hooks, the host and the store
/// (R11.6: the same loop runs in process with no service layer).
pub struct LocalAgent {
    id: EntityId,
    card: a2a::AgentCard,
    registry: Arc<Registry>,
    dispatcher: Arc<HookDispatcher>,
    host: Arc<Host>,
    store: Arc<dyn PersistenceStore>,
    prompts: Arc<PromptRegistry>,
    router: Arc<ModelRouter>,
    executor: ToolExecutor,
    system_prompt: SystemPrompt,
    builder: ContextBuilder,
    compaction: crate::config::CompactionConfig,
    limits: LoopLimits,
    on_limit: OnLimit,
    sub_task_policy: SubTaskPolicy,
    mailbox: Mailbox,
    ratios: Mutex<BTreeMap<String, TokenRatio>>,
    last_boundary: Mutex<Option<DateTime<Utc>>>,
    /// The voice a spoken turn is answered in (`[voice] voice`, R28.4).
    voice: crate::harness::models::speech::VoiceId,
}

impl core::fmt::Debug for LocalAgent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LocalAgent")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Everything a [`LocalAgent`] is built from.
#[allow(missing_debug_implementations)]
pub struct LocalAgentParts {
    /// The configuration.
    pub config: Config,
    /// The registry.
    pub registry: Arc<Registry>,
    /// The hooks.
    pub dispatcher: Arc<HookDispatcher>,
    /// The platform.
    pub host: Arc<Host>,
    /// The store.
    pub store: Arc<dyn PersistenceStore>,
    /// The prompts.
    pub prompts: Arc<PromptRegistry>,
    /// The model router (`/model`, `/effort`).
    pub router: Arc<ModelRouter>,
}

fn history_key(task: &str) -> String {
    format!("history/{task}")
}

/// The request of iteration `n` (phase 1 → 2).
fn ctx_key(task: &str, n: u32) -> String {
    format!("ctx/{task}/{n}")
}

/// The model's turn of iteration `n` (phase 2 → 3, 4).
fn turn_key(task: &str, n: u32) -> String {
    format!("turn/{task}/{n}")
}

/// One tool call's outcome in iteration `n` (phase 3 → 4).
fn tool_key(task: &str, n: u32, call: &str) -> String {
    format!("tool/{task}/{n}/{call}")
}

/// The model phase's view of a task the loop just failed.
fn stopped(outcome: IterationOutcome) -> ModelPhase {
    match outcome {
        IterationOutcome::Stopped { outcome } => ModelPhase::Stopped { outcome },
        _ => ModelPhase::Stopped {
            outcome: LoopOutcome::Failed("the model phase stopped".into()),
        },
    }
}

/// The caller tools run as: the task's reporter, else the agent itself.
fn reporter_caller(record: &TaskRecord, agent: &EntityId) -> Caller {
    record
        .participants
        .iter()
        .find(|p| p.has(Role::Reporter))
        .map_or_else(
            || Caller::system(agent.id.to_string()),
            |p| Caller {
                id: p.id.clone(),
                roles: p.roles.clone(),
                human: matches!(p.kind, ParticipantKind::Human),
                channels: BTreeSet::new(),
            },
        )
}

/// A phase payload, serialized for the store.
fn to_json<T: Serialize>(value: &T) -> Result<Vec<u8>, AgentError> {
    serde_json::to_vec(value).map_err(|e| StoreError::Corrupt(e.to_string()).into())
}

fn now(host: &Host) -> DateTime<Utc> {
    host.clock().map_or_else(|_| Utc::now(), |c| c.now())
}

/// The media types a message may carry (the card's `defaultInputModes`).
pub const INPUT_MODES: [&str; 14] = [
    "text/plain",
    "application/a2ui+json",
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "audio/wav",
    "audio/x-wav",
    "audio/mpeg",
    "audio/mp4",
    "audio/webm",
    "audio/ogg",
    "audio/pcm",
    "audio/opus",
];

/// The text of a message for the model: its text parts, a file given by URL as a note,
/// and, when there is no text, the A2UI client messages it carries (a form's answer).
fn message_text(message: &a2a::Message) -> String {
    let mut lines = Vec::new();
    let mut data = Vec::new();
    for part in &message.parts {
        match &part.content {
            a2a::PartContent::Text(t) => lines.push(t.clone()),
            a2a::PartContent::Url(url) => lines.push(format!(
                "[{} at {url}]",
                part.media_type.as_deref().unwrap_or("file")
            )),
            a2a::PartContent::Data(v) if crate::interaction::renderer::a2ui::is_a2ui(part) => {
                data.push(v.to_string());
            }
            _ => {}
        }
    }
    if lines.is_empty() {
        data.join("\n")
    } else {
        lines.join("\n")
    }
}

/// The modality of a part the model would receive inline (R28.1).
fn modality_of(part: &a2a::Part) -> Option<Modality> {
    if !matches!(part.content, a2a::PartContent::Raw(_)) {
        return None;
    }
    let media = part.media_type.as_deref()?;
    if media.starts_with("image/") {
        Some(Modality::Image)
    } else if media.starts_with("audio/") {
        Some(Modality::Audio)
    } else {
        None
    }
}

/// The image and audio parts of a message, as history entries after its text (R28.1).
fn media_turns(message: &a2a::Message) -> Vec<Turn> {
    message
        .parts
        .iter()
        .filter(|p| modality_of(p).is_some())
        .filter_map(|p| match &p.content {
            a2a::PartContent::Raw(bytes) => Some(Turn::new(HistoryEntry::UserMedia {
                media_type: p.media_type.clone().unwrap_or_default(),
                data: Bytes::from(bytes.clone()),
                name: p.filename.clone(),
            })),
            _ => None,
        })
        .collect()
}

/// A tool's artifact as a stored event (R28.2).
fn artifact_event(
    task: &TaskRecord,
    attempt: Attempt,
    ordinal: u32,
    artifact: a2a::Artifact,
) -> StoredEvent {
    StoredEvent {
        id: StoredEvent::event_id(&task.id, attempt, "artifact", ordinal),
        seq: 0,
        at: task.updated_at,
        attempt,
        visibility: Visibility::All,
        response: a2a::StreamResponse::ArtifactUpdate(a2a::TaskArtifactUpdateEvent {
            task_id: task.id.clone(),
            context_id: task.context_id.clone(),
            artifact,
            append: None,
            last_chunk: Some(true),
            metadata: None,
        }),
    }
}

fn message_event(
    task: &TaskRecord,
    attempt: Attempt,
    ordinal: u32,
    message: a2a::Message,
    at: DateTime<Utc>,
) -> StoredEvent {
    StoredEvent {
        id: StoredEvent::event_id(&task.id, attempt, "message", ordinal),
        seq: 0,
        at,
        attempt,
        visibility: Visibility::All,
        response: a2a::StreamResponse::Message(message),
    }
}

fn status_event(task: &TaskRecord, attempt: Attempt, ordinal: u32) -> StoredEvent {
    StoredEvent {
        id: StoredEvent::event_id(&task.id, attempt, "status", ordinal),
        seq: 0,
        at: task.updated_at,
        attempt,
        visibility: Visibility::All,
        response: a2a::StreamResponse::StatusUpdate(task.status_update()),
    }
}

fn task_meta<'a>(message: &'a a2a::Message, key: &str) -> Option<&'a Value> {
    message.metadata.as_ref()?.get(TASK_EXT)?.get(key)
}

impl LocalAgent {
    /// An agent from its parts; the card is built from `[agent]`.
    #[must_use]
    pub fn new(parts: LocalAgentParts) -> Self {
        let LocalAgentParts {
            config,
            registry,
            dispatcher,
            host,
            store,
            prompts,
            router,
        } = parts;
        let locale = config.locale.clone().unwrap_or_else(|| "en".into());
        let card = a2a::AgentCard {
            name: config.agent.name.clone(),
            description: config.agent.description.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            supported_interfaces: Vec::new(),
            capabilities: a2a::AgentCapabilities {
                streaming: Some(true),
                ..a2a::AgentCapabilities::default()
            },
            // Text both ways, and A2UI (R25.4): the ask's data part out, the client
            // action's data part back; the SDK refuses a part whose media type the
            // card does not list (exact match). Images and audio are listed too: the
            // harness carries them, and the selected model's check refuses them by
            // name when it cannot take them (R28.1).
            default_input_modes: INPUT_MODES.iter().map(|m| (*m).to_owned()).collect(),
            default_output_modes: vec![
                "text/plain".into(),
                crate::interaction::renderer::a2ui::MEDIA_TYPE.into(),
            ],
            skills: Vec::new(),
            provider: None,
            documentation_url: None,
            icon_url: None,
            security_schemes: None,
            security_requirements: None,
            signatures: None,
        };
        let executor = ToolExecutor::new(
            Arc::clone(&registry),
            Arc::clone(&dispatcher),
            Arc::clone(&host),
        )
        .with_result_limit(config.tools.max_result_bytes, OversizePolicy::Truncate);
        let limits = LoopLimits {
            max_iterations: config.r#loop.max_iterations,
            max_total_tokens: config.r#loop.max_tokens,
            wall_clock: config.r#loop.wall_clock.map(Into::into),
        };
        let on_limit = if config.r#loop.on_limit == "fail" {
            OnLimit::Fail
        } else {
            OnLimit::InputRequired
        };
        Self {
            id: EntityId::new(config.agent.name.clone()),
            card,
            system_prompt: SystemPrompt::new(
                Arc::clone(&prompts),
                config.agent.clone(),
                locale.clone(),
            ),
            builder: ContextBuilder::new(
                ContextTemplate::from_order(&config.context.order),
                Arc::clone(&prompts),
                locale,
            ),
            compaction: config.context.compaction.clone(),
            registry,
            dispatcher,
            host,
            store,
            prompts,
            router,
            executor,
            limits,
            on_limit,
            sub_task_policy: SubTaskPolicy::default(),
            mailbox: Mailbox::default(),
            ratios: Mutex::new(BTreeMap::new()),
            last_boundary: Mutex::new(None),
            voice: crate::harness::models::speech::VoiceId(
                config.voice.voice.clone().unwrap_or_else(|| {
                    crate::harness::models::speech::openai::DEFAULT_VOICE.into()
                }),
            ),
        }
    }

    /// The policy for sub-tasks of a task that reached a terminal state (R12.4).
    #[must_use]
    pub fn with_sub_task_policy(mut self, policy: SubTaskPolicy) -> Self {
        self.sub_task_policy = policy;
        self
    }

    /// The limits, replacing `[loop]`'s.
    #[must_use]
    pub fn with_limits(mut self, limits: LoopLimits, on_limit: OnLimit) -> Self {
        self.limits = limits;
        self.on_limit = on_limit;
        self
    }

    /// The in-memory inbox.
    #[must_use]
    pub fn mailbox(&self) -> &Mailbox {
        &self.mailbox
    }

    /// The hook dispatcher every operation of this agent runs through.
    #[must_use]
    pub fn hooks(&self) -> &Arc<HookDispatcher> {
        &self.dispatcher
    }

    /// The store.
    #[must_use]
    pub fn store(&self) -> &Arc<dyn PersistenceStore> {
        &self.store
    }

    /// The platform.
    #[must_use]
    pub fn host(&self) -> &Arc<Host> {
        &self.host
    }

    /// When the loop last reached a boundary, for the heartbeat's liveness (R23.2).
    #[must_use]
    pub fn last_boundary(&self) -> Option<DateTime<Utc>> {
        *self
            .last_boundary
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn cx(&self, task: &TaskRecord) -> HookCx {
        let mut cx = HookCx::new(Arc::clone(&self.registry), Arc::clone(&self.host));
        cx.task = Some(task.id.clone());
        cx.context_id = Some(task.context_id.clone());
        cx.traceparent.clone_from(&task.traceparent);
        cx
    }

    async fn load_history(&self, task: &TaskId) -> Result<Vec<Turn>, AgentError> {
        Ok(match self.store.get_blob(&history_key(task)).await? {
            Some(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| StoreError::Corrupt(e.to_string()))?
            }
            None => Vec::new(),
        })
    }

    fn history_blob(task: &str, history: &[Turn]) -> BlobWrite {
        BlobWrite {
            key: history_key(task),
            bytes: Bytes::from(serde_json::to_vec(history).unwrap_or_default()),
        }
    }

    async fn record(&self, task: &TaskId) -> Result<TaskRecord, AgentError> {
        self.store
            .get_task(task)
            .await?
            .ok_or_else(|| AgentError::Unsupported(format!("task {task}: not found")))
    }

    async fn change(
        &self,
        task: TaskRecord,
        transition: Transition,
    ) -> Result<TaskRecord, AgentError> {
        let cx = self.cx(&task);
        let at = now(&self.host);
        let changed = transition_with_hooks(
            &self.dispatcher,
            &cx,
            StateChange {
                task,
                transition,
                at,
            },
        )
        .await?;
        Ok(changed.task)
    }

    /// Creates the task a first message addresses (R20.6: the one implicit creation
    /// A2A defines) and links it to its parent when the message names one (R12.3).
    async fn create_task(
        &self,
        request: &AgentRequest,
        text: String,
        at: DateTime<Utc>,
        shell_version: Option<u64>,
    ) -> Result<TaskRecord, AgentError> {
        let caller = &request.call.caller;
        let mode = match task_meta(&request.message, "mode").and_then(Value::as_str) {
            Some("long-running") => TaskMode::LongRunning,
            _ => TaskMode::OneShot,
        };
        let mut roles: BTreeSet<Role> = caller.roles.clone();
        roles.insert(Role::Reporter);
        let draft = TaskDraft {
            id: request.call.task_id.clone(),
            context_id: request.call.context_id.clone(),
            name: text.chars().take(60).collect(),
            r#type: None,
            goal: text.clone(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            mode,
            participants: vec![
                Participant {
                    id: caller.id.clone(),
                    display: caller.id.clone(),
                    kind: ParticipantKind::Human,
                    roles,
                },
                Participant {
                    id: self.id.id.to_string(),
                    display: self.card.name.clone(),
                    kind: ParticipantKind::Agent(self.id.clone()),
                    roles: BTreeSet::from([Role::Assignee]),
                },
            ],
            parent_tasks: task_meta(&request.message, "parent")
                .and_then(Value::as_str)
                .map(|p| {
                    vec![crate::harness::core_loop::task::TaskRef {
                        task: p.to_owned(),
                        context_id: None,
                    }]
                })
                .unwrap_or_default(),
            reference_task_ids: request
                .message
                .reference_task_ids
                .clone()
                .unwrap_or_default(),
            model: self.router.selection(),
        };
        let traceparent = crate::app_service::o11y::trace::for_task(request.traceparent.as_deref());
        let mut cx = HookCx::new(Arc::clone(&self.registry), Arc::clone(&self.host));
        cx.task = Some(draft.id.clone());
        cx.context_id = Some(draft.context_id.clone());
        cx.traceparent = Some(traceparent.clone());
        let mut record = self
            .dispatcher
            .run::<crate::harness::core_loop::task::TaskCreate, _, _>(
                &cx,
                draft,
                move |d| async move { Ok(TaskRecord::new(d, at)) },
            )
            .await?;
        record.traceparent.get_or_insert(traceparent);
        let mut history = vec![Turn::new(HistoryEntry::User { text })];
        history.extend(media_turns(&request.message));
        let attempt = Attempt { iteration: 0, n: 1 };
        let mut commit = TaskCommit::of(record.clone()).with_events([
            message_event(&record, attempt, 0, request.message.clone(), at),
            status_event(&record, attempt, 0),
        ]);
        commit.expect_version = Some(shell_version.unwrap_or(0));
        commit.inbox_applied.extend(request.inbox_event_id.clone());
        commit.blobs.push(Self::history_blob(&record.id, &history));
        self.store.commit(commit).await?;
        for parent in &record.parent_tasks {
            if let Some(mut p) = self.store.get_task(&parent.task).await? {
                p.link_sub_task(crate::harness::core_loop::task::TaskRef {
                    task: record.id.clone(),
                    context_id: Some(record.context_id.clone()),
                });
                self.store.commit(TaskCommit::of(p)).await?;
            }
        }
        self.record(&record.id).await
    }

    /// The message as the model will take it: `voice/v1` audio transcribed through the
    /// registered `SpeechToText` when the selected model cannot hear (R28.3), each
    /// transcript a text part that keeps the audio's `voice/v1` metadata. A model that
    /// hears, or a harness without speech-to-text, gets the message unchanged.
    ///
    /// # Errors
    ///
    /// [`AgentError::Model`] when transcription fails.
    async fn hear(&self, message: &a2a::Message) -> Result<a2a::Message, AgentError> {
        use crate::harness::models::speech::{SpeechToText, TranscriptEvent, audio_of};
        if !message.parts.iter().any(|p| audio_of(p).is_some()) {
            return Ok(message.clone());
        }
        let hears = self.router.current().is_ok_and(|llm| {
            llm.capabilities()
                .input_modalities
                .contains(&Modality::Audio)
        });
        let Ok(stt) = self.registry.latest::<dyn SpeechToText>("default") else {
            return Ok(message.clone());
        };
        if hears {
            return Ok(message.clone());
        }
        let mut transcribed = message.clone();
        for part in &mut transcribed.parts {
            let Some((meta, bytes)) = audio_of(part) else {
                continue;
            };
            let audio =
                crate::runtime::boxed_stream(futures::stream::once(async move { Ok(bytes) }));
            let mut events = stt
                .transcribe(crate::harness::models::speech::openai::PCM24K, audio)
                .await
                .map_err(|e| AgentError::Model(e.to_string()))?;
            let mut deltas = String::new();
            let mut text = None;
            while let Some(event) = futures::StreamExt::next(&mut events).await {
                match event.map_err(|e| AgentError::Model(e.to_string()))? {
                    TranscriptEvent::Delta { text: d } => deltas.push_str(&d),
                    TranscriptEvent::Final { text: t } => text = Some(t),
                }
            }
            let mut transcript = a2a::Part::text(text.unwrap_or(deltas));
            transcript.metadata = Some(HashMap::from([(
                crate::harness::models::speech::VOICE_EXT.to_owned(),
                serde_json::to_value(&meta).unwrap_or(Value::Null),
            )]));
            *part = transcript;
        }
        Ok(transcribed)
    }

    /// The reply in speech, when the participant's last turn was spoken and a
    /// `TextToSpeech` is registered (R28.4); `None` otherwise. A synthesis failure is
    /// logged and the text reply stands alone.
    async fn speak(
        &self,
        record: &TaskRecord,
        attempt: Attempt,
        reply: &a2a::Message,
    ) -> Option<a2a::Artifact> {
        use crate::harness::models::speech::{
            AudioChunk, TextToSpeech, VOICE_EXT, VoiceMeta, audio_part,
        };
        let tts = self.registry.latest::<dyn TextToSpeech>("default").ok()?;
        let events = self
            .store
            .events(&record.id, 1, usize::MAX, &Caller::system("voice"))
            .await
            .ok()?;
        let last_user = events.iter().rev().find_map(|e| match &e.response {
            a2a::StreamResponse::Message(m) if m.role == a2a::Role::User => Some(m),
            _ => None,
        })?;
        let spoken = last_user.parts.iter().any(|p| {
            p.metadata
                .as_ref()
                .is_some_and(|m| m.contains_key(VOICE_EXT))
        });
        let text = message_text(reply);
        if !spoken || text.trim().is_empty() {
            return None;
        }
        let stream = match tts.synthesise(text, self.voice.clone()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(task = %record.id, error = %e, "speech synthesis failed; the reply stays text");
                return None;
            }
        };
        let chunks: Vec<Result<AudioChunk, _>> = futures::StreamExt::collect(stream).await;
        let mut data = Vec::new();
        for chunk in chunks {
            match chunk {
                Ok(c) => data.extend_from_slice(&c.data),
                Err(e) => {
                    tracing::warn!(task = %record.id, error = %e, "speech synthesis failed; the reply stays text");
                    return None;
                }
            }
        }
        let chunk = AudioChunk {
            format: crate::harness::models::speech::openai::PCM24K,
            data: Bytes::from(data),
            seq: 1,
        };
        let meta = VoiceMeta {
            seq: 1,
            attempt,
            format: "pcm16@24k".into(),
            is_final: true,
            session: record.id.clone(),
        };
        Some(a2a::Artifact {
            artifact_id: format!("speech-{}-{}", attempt.iteration, attempt.n),
            name: Some("reply.pcm".into()),
            description: Some("the reply, spoken".into()),
            parts: vec![audio_part(&chunk, &meta)],
            metadata: None,
            extensions: None,
        })
    }

    /// Creates the task for a first message, or applies a follow-up to an existing one
    /// (R20.6, R21.5, R21.6). Returns the record and whether the loop should run now.
    ///
    /// # Errors
    ///
    /// [`AgentError::Unsupported`] for a terminal task or a context mismatch,
    /// [`AgentError::UnsupportedModality`] for media the model cannot take.
    pub async fn open_task(
        &self,
        request: &AgentRequest,
    ) -> Result<(TaskRecord, bool), AgentError> {
        // Speech the model cannot hear is transcribed first (R28.3); the log keeps the
        // transcript, marked as spoken.
        let heard = AgentRequest {
            message: self.hear(&request.message).await?,
            ..request.clone()
        };
        let request = &heard;
        // Media the selected model cannot take is refused before anything is stored,
        // naming the modality (R28.1).
        if let Some(missing) = request
            .message
            .parts
            .iter()
            .filter_map(modality_of)
            .find(|m| {
                self.router
                    .current()
                    .is_ok_and(|llm| !llm.capabilities().input_modalities.contains(m))
            })
        {
            return Err(AgentError::UnsupportedModality {
                modality: missing,
                model: self.router.selection().model,
            });
        }
        let text = message_text(&request.message);
        let at = now(&self.host);
        let caller = &request.call.caller;
        match self.store.get_task(&request.call.task_id).await? {
            None => Ok((self.create_task(request, text, at, None).await?, true)),
            // A shell record the A2A server's task store created ahead of the executor
            // (no participants yet): adopt it as the first message's task.
            Some(record)
                if record.participants.is_empty() && record.state == a2a::TaskState::Submitted =>
            {
                Ok((
                    self.create_task(request, text, at, Some(record.version))
                        .await?,
                    true,
                ))
            }
            Some(record) if record.context_id != request.call.context_id => {
                Err(AgentError::Unsupported(format!(
                    "task {} belongs to context {}, not {}",
                    record.id, record.context_id, request.call.context_id
                )))
            }
            Some(record) if record.is_terminal() => Err(AgentError::Unsupported(format!(
                "task {} is {:?}; start a new task in the same context with referenceTaskIds",
                record.id, record.state
            ))),
            Some(record)
                if task_meta(&request.message, "transition").and_then(Value::as_str)
                    == Some("complete") =>
            {
                self.complete(&record.id, &caller.id).await?;
                Ok((self.record(&record.id).await?, false))
            }
            Some(record) if self.mailbox.is_running(&record.id) => {
                self.mailbox.deliver(
                    &record.id,
                    InboxEvent {
                        id: request.message.message_id.clone(),
                        target: crate::app_service::inbox::EventTarget::Task(record.id.clone()),
                        kind: a2a::StreamResponse::Message(request.message.clone()),
                        status: crate::app_service::inbox::InboxStatus::Received,
                        sender: caller.clone(),
                        received_at: at,
                        delivery: crate::app_service::inbox::DeliveryKind::Interrupt,
                        extensions: request.extensions.clone(),
                        traceparent: request
                            .traceparent
                            .clone()
                            .or_else(|| record.traceparent.clone()),
                    },
                );
                Ok((record, false))
            }
            Some(record) => self.continue_task(request, record, text, at).await,
        }
    }

    /// A follow-up on an open task (R21.5): a channel reply resolves the pending call it
    /// answers, after a membership check (R18.5, abuse 2); any other message is the next
    /// turn.
    async fn continue_task(
        &self,
        request: &AgentRequest,
        mut record: TaskRecord,
        text: String,
        at: DateTime<Utc>,
    ) -> Result<(TaskRecord, bool), AgentError> {
        let caller = &request.call.caller;
        let mut history = self.load_history(&record.id).await?;
        let mut visibility = Visibility::All;
        let meta = crate::harness::channels::ChannelsMeta::of(&request.message);
        let resolved = match &meta {
            Some(meta) => {
                let channel =
                    crate::harness::channels::Channel::load(self.store.as_ref(), &meta.channel)
                        .await?;
                match channel {
                    Some(c) if c.task == record.id && c.admits(&caller.id) => {
                        visibility = Visibility::Members(c.id.clone());
                        let ask = meta.reply_to.clone().unwrap_or_else(|| meta.ask.clone());
                        record.pending_calls.iter().position(
                            |p| matches!(&p.resume_on, ResumeOn::Ask { ask: a } if *a == ask),
                        )
                    }
                    _ => return Err(AgentError::NotFound(record.id.clone())),
                }
            }
            // A plain message while a participant is awaited answers the oldest ask.
            None if matches!(
                record.sub_state,
                Some(crate::harness::core_loop::task::SubState::AwaitingParticipant { .. })
            ) =>
            {
                record
                    .pending_calls
                    .iter()
                    .position(|p| matches!(p.resume_on, ResumeOn::Ask { .. }))
            }
            None => None,
        };
        if let Some(i) = resolved {
            let pending = record.pending_calls.remove(i);
            history.push(Turn::new(HistoryEntry::ToolResult {
                call_id: pending.call_id,
                text: text.clone(),
            }));
        } else {
            history.push(Turn::new(HistoryEntry::User { text }));
            history.extend(media_turns(&request.message));
        }
        let attempt = Attempt {
            iteration: record.iteration,
            n: 1,
        };
        let ordinal = u32::try_from(history.len()).unwrap_or(u32::MAX);
        let record = if record.state == a2a::TaskState::InputRequired {
            self.change(record, Transition::Resume).await?
        } else {
            record
        };
        let mut inbound = message_event(&record, attempt, ordinal, request.message.clone(), at);
        inbound.visibility = visibility;
        let mut commit = TaskCommit::of(record.clone())
            .with_events([inbound, status_event(&record, attempt, ordinal)]);
        commit.pending_calls.clone_from(&record.pending_calls);
        commit.inbox_applied.extend(request.inbox_event_id.clone());
        commit.blobs.push(Self::history_blob(&record.id, &history));
        self.store.commit(commit).await?;
        Ok((self.record(&record.id).await?, true))
    }

    /// One line per registered sub-agent (R17.2); the harness itself is not listed.
    fn sub_agent_index(&self) -> String {
        self.registry
            .list::<dyn Agent>()
            .iter()
            .filter(|a| a.entity_id() != &self.id)
            .map(|a| {
                let card = a.card();
                let skills: Vec<&str> = card.skills.iter().map(|s| s.name.as_str()).collect();
                if skills.is_empty() {
                    format!("- {}: {}", a.entity_id().id, card.description)
                } else {
                    format!(
                        "- {}: {} (skills: {})",
                        a.entity_id().id,
                        card.description,
                        skills.join(", ")
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn ratio_for(&self, model: &str) -> TokenRatio {
        self.ratios
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(model)
            .cloned()
            .unwrap_or_default()
    }

    async fn inputs(
        &self,
        record: &TaskRecord,
        history: Vec<Turn>,
    ) -> Result<ContextInputs, AgentError> {
        let cx = self.cx(record);
        let system_prompt = self
            .system_prompt
            .assemble(&self.dispatcher, &cx, &record.participants, &[])
            .await?;
        let skills_index = self
            .registry
            .list::<Skill>()
            .iter()
            .map(|s| s.index_line())
            .collect::<Vec<_>>()
            .join("\n");
        // A plan the task names, else the one `plan.update` keeps for it (R13.4).
        let plan_id = record
            .plan
            .clone()
            .unwrap_or_else(|| crate::harness::core_loop::plan::tools::plan_id(&record.id));
        let plan = self.store.get_plan(&plan_id).await?;
        let state = self.store.get_state(&record.id).await?.unwrap_or_default();
        Ok(ContextInputs {
            system_prompt,
            system_extensions: Prompt::default(),
            skills_index,
            sub_agent_index: self.sub_agent_index(),
            task: record.clone(),
            plan,
            state,
            history,
            tools: self.executor.descriptors(),
            custom: BTreeMap::new(),
        })
    }

    /// One iteration of the loop (design.md § Agent and the core loop): the four phases
    /// in order, each reading what the one before persisted (R22.2: the phases are the
    /// activities durable execution schedules; `none` runs them here in sequence).
    ///
    /// # Errors
    ///
    /// Infrastructure failures (the store, a model error worth retrying); everything the
    /// loop decides is an [`IterationOutcome`].
    pub async fn run_iteration(
        &self,
        task: &TaskId,
        n: u32,
    ) -> Result<IterationOutcome, AgentError> {
        let attempt = Attempt { iteration: n, n: 1 };
        if let Begun::Stopped { outcome } = self.iteration_begin(task, n).await? {
            return Ok(IterationOutcome::Stopped { outcome });
        }
        let calls = match self.iteration_model(task, n, attempt).await? {
            ModelPhase::Stopped { outcome } => return Ok(IterationOutcome::Stopped { outcome }),
            ModelPhase::Turn { calls, .. } => calls,
        };
        // tool.execute ×n, concurrently, results read back in issue order (R8.2).
        for result in futures::future::join_all(
            calls
                .iter()
                .map(|call| self.iteration_tool(task, n, attempt, call)),
        )
        .await
        {
            result?;
        }
        self.iteration_finish(task, n, attempt).await
    }

    /// Phase 1, `build_context`: the boundary (events, signals, limits), then the context
    /// with compaction when needed; the request lands in `ctx/<task>/<n>` and the history
    /// in its blob, so the next phase needs only the keys (store-keyed payloads, R22.2).
    ///
    /// # Errors
    ///
    /// Infrastructure failures.
    #[allow(clippy::too_many_lines)] // one phase, read top to bottom
    pub async fn iteration_begin(&self, task: &TaskId, n: u32) -> Result<Begun, AgentError> {
        let mut record = self.record(task).await?;
        let attempt = Attempt { iteration: n, n: 1 };
        if record.state == a2a::TaskState::Submitted {
            record = self.change(record, Transition::Start).await?;
            self.store
                .commit(
                    TaskCommit::of(record.clone()).with_events([status_event(&record, attempt, 0)]),
                )
                .await?;
            record = self.record(task).await?;
        }
        if record.state == a2a::TaskState::InputRequired {
            // A waiting task wakes only for a message the inbox holds for it (R23.1).
            let has_message = self.store.pending_inbox(task).await?.iter().any(|e| {
                !record.applied_events.contains(&e.id)
                    && matches!(e.kind, a2a::StreamResponse::Message(_))
            });
            if !has_message {
                return Ok(Begun::Stopped {
                    outcome: outcome_of(&record),
                });
            }
            record = self.change(record, Transition::Resume).await?;
            let cx = self.cx(&record);
            let mut history = self.load_history(task).await?;
            self.apply_pending_inbox(&mut record, &mut history, &cx)
                .await?;
            if record.state != a2a::TaskState::Working {
                let mut commit = TaskCommit::of(record.clone());
                commit.blobs.push(Self::history_blob(task, &history));
                self.store.commit(commit).await?;
                record = self.record(task).await?;
            }
        }
        if record.state != a2a::TaskState::Working {
            return Ok(Begun::Stopped {
                outcome: outcome_of(&record),
            });
        }
        let cx = self.cx(&record);
        let mut history = self.load_history(task).await?;

        // Boundary: events that arrived since the last step, then signals (R11.5). The
        // mailbox is a wake-up; an event the inbox also persisted is applied from the
        // store, once (R20.5).
        *self
            .last_boundary
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(now(&self.host));
        let persisted: Vec<String> = self
            .store
            .pending_inbox(task)
            .await?
            .into_iter()
            .map(|e| e.id)
            .collect();
        for event in self.mailbox.drain(task) {
            if persisted.contains(&event.id) || record.applied_events.contains(&event.id) {
                continue;
            }
            if let a2a::StreamResponse::Message(m) = &event.kind {
                history.push(Turn::new(HistoryEntry::User {
                    text: message_text(m),
                }));
                history.extend(media_turns(m));
            }
        }
        self.apply_pending_inbox(&mut record, &mut history, &cx)
            .await?;
        if let Some(signal) = self.mailbox.take_signal(task) {
            return match self.stop(record, history, attempt, signal).await? {
                IterationOutcome::Stopped { outcome } => Ok(Begun::Stopped { outcome }),
                _ => Ok(Begun::Stopped {
                    outcome: outcome_of(&self.record(task).await?),
                }),
            };
        }

        // context.build (with compaction when the estimate is above the threshold).
        let llm = self.router.current()?;
        let alias = self.router.selection().model;
        let ratio = self.ratio_for(&alias);
        let mut inputs = self.inputs(&record, history.clone()).await?;
        let mut built = self
            .builder
            .build_with_hooks(&self.dispatcher, &cx, &inputs, &ratio)
            .await?;
        let window = llm.capabilities().max_window_tokens;
        let compactor = Compactor::new(
            self.compaction.clone(),
            self.builder.template().clone(),
            Arc::clone(&self.prompts),
            Arc::clone(&llm),
        );
        if compactor.needed(&built.estimate, window) {
            let compacted = compactor
                .compact(
                    &self.dispatcher,
                    &cx,
                    CompactionRequest {
                        task,
                        goal: &record.goal,
                        history: &history,
                        estimate: &built.estimate,
                        window,
                        ratio: &ratio,
                    },
                )
                .await?;
            if let Some(summary) = &compacted.summary {
                self.store
                    .put_blob(&format!("summary/{task}/{n}"), Bytes::from(summary.clone()))
                    .await?;
            }
            history = compacted.history;
            inputs.history.clone_from(&history);
            built = self
                .builder
                .build_with_hooks(&self.dispatcher, &cx, &inputs, &ratio)
                .await?;
        }
        let chars = request_chars(&built.request);
        let ctx_key = ctx_key(task, n);
        self.store
            .put_blob(&ctx_key, Bytes::from(to_json(&built.request)?))
            .await?;
        let history_blob = Self::history_blob(task, &history);
        self.store
            .put_blob(&history_blob.key, history_blob.bytes)
            .await?;
        Ok(Begun::Context {
            ctx_key,
            chars,
            model: alias,
        })
    }

    /// Phase 2, `call_model`: the request from `ctx/<task>/<n>` to the model, then
    /// `response.parse`; the turn (with the parsed calls) lands in `turn/<task>/<n>`.
    ///
    /// # Errors
    ///
    /// A retryable provider error that ran out of attempts, or the store: infrastructure
    /// failures durable execution may retry (R22.3). A non-retryable provider error or a
    /// veto fails the task and is a [`ModelPhase::Stopped`].
    pub async fn iteration_model(
        &self,
        task: &TaskId,
        n: u32,
        attempt: Attempt,
    ) -> Result<ModelPhase, AgentError> {
        let record = self.record(task).await?;
        if record.state != a2a::TaskState::Working {
            return Ok(ModelPhase::Stopped {
                outcome: outcome_of(&record),
            });
        }
        let cx = self.cx(&record);
        let request: ModelRequest = self.load_json(&ctx_key(task, n), "context").await?;
        let chars = request_chars(&request);
        let llm = self.router.current()?;
        let alias = self.router.selection().model;
        let mut ratio = self.ratio_for(&alias);

        // model.call
        let model_error: Arc<Mutex<Option<ModelError>>> = Arc::default();
        let sink = Arc::clone(&model_error);
        let turn = match self
            .dispatcher
            .run::<ModelCall, _, _>(&cx, request, move |request| async move {
                let outcome = async {
                    let events = llm.stream(request).await?;
                    collect(events, Some(Instant::now())).await
                }
                .await;
                outcome.map_err(|e| {
                    let text = e.to_string();
                    *sink.lock().unwrap_or_else(PoisonError::into_inner) = Some(e);
                    HarnessError::Internal(text)
                })
            })
            .await
        {
            Ok(turn) => turn,
            Err(HarnessError::Vetoed(v)) => {
                return Ok(stopped(
                    self.fail_task(record, attempt, "vetoed", v.reason).await?,
                ));
            }
            Err(e) => {
                let provider = model_error
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                return match provider {
                    // A non-retryable provider error is the task's typed error (R9.6);
                    // a retryable one that ran out of attempts is an infrastructure
                    // failure durable execution may retry.
                    Some(err @ (ModelError::NonRetryable { .. } | ModelError::Decode(_))) => {
                        Ok(stopped(
                            self.fail_task(record, attempt, "model", err.to_string())
                                .await?,
                        ))
                    }
                    _ => Err(e.into()),
                };
            }
        };
        if let Some(usage) = &turn.usage {
            ratio.learn(chars, usage);
            self.ratios
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(alias, ratio);
        }

        // response.parse: hooks decide the calls the model's turn made.
        let calls = self
            .dispatcher
            .run::<ResponseParse, _, _>(&cx, turn.clone(), |t| async move { Ok(t.tool_calls) })
            .await?;
        let mut turn = turn;
        turn.tool_calls = calls;
        let turn_key = turn_key(task, n);
        self.store
            .put_blob(&turn_key, Bytes::from(to_json(&turn)?))
            .await?;
        Ok(ModelPhase::Turn {
            turn_key,
            calls: turn.tool_calls.iter().map(|c| c.id.clone()).collect(),
            stop: turn.stop,
            usage: turn.usage,
        })
    }

    /// Phase 3, `execute_tool`: one of the turn's calls, under the task's cancellation
    /// token and the idempotency key `(task, n, call)`; the outcome lands in
    /// `tool/<task>/<n>/<call>`.
    ///
    /// # Errors
    ///
    /// The store, or a call the turn does not have.
    pub async fn iteration_tool(
        &self,
        task: &TaskId,
        n: u32,
        attempt: Attempt,
        call_id: &str,
    ) -> Result<(), AgentError> {
        let record = self.record(task).await?;
        let cx = self.cx(&record);
        let turn: ModelTurn = self.load_json(&turn_key(task, n), "turn").await?;
        let call = turn
            .tool_calls
            .into_iter()
            .find(|c| c.id == call_id)
            .ok_or_else(|| {
                AgentError::Unsupported(format!("task {task}: no call {call_id} in turn {n}"))
            })?;
        let caller = reporter_caller(&record, &self.id);
        let token = self.mailbox.token(task);
        let executed = self
            .executor
            .execute_one(&cx, task, attempt, &caller, &token, call)
            .await;
        self.store
            .put_blob(
                &tool_key(task, n, call_id),
                Bytes::from(to_json(&executed)?),
            )
            .await?;
        Ok(())
    }

    /// Phase 4, `finish_iteration` (`response.deliver` or the tools' fold): the turn and
    /// the tool outcomes read back by key, the history extended, one commit (R21.1,
    /// R21.3, R8.2).
    ///
    /// # Errors
    ///
    /// The store, or a missing turn or tool outcome.
    #[allow(clippy::too_many_lines)] // one phase, read top to bottom
    pub async fn iteration_finish(
        &self,
        task: &TaskId,
        n: u32,
        attempt: Attempt,
    ) -> Result<IterationOutcome, AgentError> {
        let mut record = self.record(task).await?;
        if record.state != a2a::TaskState::Working {
            return Ok(IterationOutcome::Stopped {
                outcome: outcome_of(&record),
            });
        }
        let cx = self.cx(&record);
        let mut history = self.load_history(task).await?;
        let turn: ModelTurn = self.load_json(&turn_key(task, n), "turn").await?;
        history.push(Turn::new(HistoryEntry::Assistant {
            text: turn.text.clone(),
            tool_calls: turn.tool_calls.clone(),
        }));
        let mut reply =
            a2a::Message::new(a2a::Role::Agent, vec![a2a::Part::text(turn.text.clone())]);
        reply.task_id = Some(task.clone());
        reply.context_id = Some(record.context_id.clone());
        let calls = turn.tool_calls;
        record.iteration = n;

        if calls.is_empty() {
            // response.deliver and the close-or-keep-open policy (R11.3, R21.1, R21.3).
            let delivery = self
                .dispatcher
                .run::<ResponseDeliver, _, _>(
                    &cx,
                    Delivery {
                        task: task.clone(),
                        message: reply,
                    },
                    |d| async move { Ok(d) },
                )
                .await?;
            record.status_message = Some(delivery.message.clone());
            let record = self.change(record, Transition::Delivered).await?;
            // A spoken turn is answered in speech too (R28.4): the reply's text,
            // synthesised, as an audio artifact next to the message.
            let spoken = self.speak(&record, attempt, &delivery.message).await;
            let mut commit = TaskCommit::of(record.clone()).with_events([
                message_event(&record, attempt, 0, delivery.message, now(&self.host)),
                status_event(&record, attempt, 1),
            ]);
            if let Some(artifact) = spoken {
                commit
                    .events
                    .push(artifact_event(&record, attempt, 0, artifact));
            }
            commit.blobs.push(Self::history_blob(task, &history));
            self.store.commit(commit).await?;
            if record.is_terminal() {
                self.finish_sub_tasks(&record).await?;
            }
            return Ok(IterationOutcome::Delivered);
        }

        // The tools' outcomes, in issue order (R8.2).
        let mut pending: Vec<PendingCall> = Vec::new();
        let mut question: Option<a2a::Message> = None;
        let mut artifacts: Vec<a2a::Artifact> = Vec::new();
        for call in &calls {
            let executed: Executed = self
                .load_json(&tool_key(task, n, &call.id), "tool outcome")
                .await?;
            match executed {
                Executed::Result { result } => {
                    history.push(Turn::new(HistoryEntry::ToolResult {
                        call_id: result.call_id.clone(),
                        text: result.render(),
                    }));
                    artifacts.extend(result.artifacts);
                }
                Executed::Suspended {
                    pending: p,
                    message,
                } => {
                    if let Some(m) = message {
                        question = Some(*m);
                    }
                    pending.push(p);
                }
            }
        }
        // A tool may have linked a sub-task on the record meanwhile (`agent.call`,
        // `participant.delegate`): take its links and version, keep this iteration's changes.
        let fresh = self.record(task).await?;
        record.version = fresh.version;
        record.sub_tasks = fresh.sub_tasks;
        let mut commit_events = vec![message_event(&record, attempt, 0, reply, now(&self.host))];
        commit_events.extend(artifacts.into_iter().enumerate().map(|(i, a)| {
            artifact_event(&record, attempt, u32::try_from(i).unwrap_or(u32::MAX), a)
        }));
        if let Some(first) = pending.first().cloned() {
            let ask = match &first.resume_on {
                ResumeOn::Ask { ask } => ask.clone(),
                ResumeOn::SubTask { task } => task.clone(),
            };
            record.pending_calls.clone_from(&pending);
            let channel = question
                .as_ref()
                .and_then(crate::harness::channels::ChannelsMeta::of);
            record.status_message = question;
            let record = self
                .change(record, Transition::Ask { ask: ask.clone() })
                .await?;
            match channel.and_then(|c| (!c.to.is_empty()).then_some(c)) {
                // A question for some participants: members see it, the rest see the
                // state change only (R18.5, abuse 2).
                Some(meta) => {
                    let stub = crate::harness::channels::Channel {
                        id: meta.channel.clone(),
                        task: record.id.clone(),
                        members: meta.to.iter().cloned().collect(),
                        created_by: self.id.id.to_string(),
                        open: true,
                    };
                    let mut redacted = status_event(&record, attempt, 1);
                    redacted.response = a2a::StreamResponse::StatusUpdate(
                        crate::harness::channels::redacted_status(&record, &stub),
                    );
                    commit_events.push(redacted);
                    let mut full = status_event(&record, attempt, 2);
                    full.visibility = Visibility::Members(meta.channel.clone());
                    commit_events.push(full);
                }
                None => commit_events.push(status_event(&record, attempt, 1)),
            }
            let mut commit = TaskCommit::of(record.clone()).with_events(commit_events);
            commit.pending_calls = pending;
            commit.blobs.push(Self::history_blob(task, &history));
            self.store.commit(commit).await?;
            return Ok(IterationOutcome::Suspended { ask });
        }
        let mut commit = TaskCommit::of(record.clone()).with_events(commit_events);
        commit.blobs.push(Self::history_blob(task, &history));
        self.store.commit(commit).await?;
        if let Some(signal) = self.mailbox.take_signal(task) {
            let record = self.record(task).await?;
            return self.stop(record, history, attempt, signal).await;
        }
        Ok(IterationOutcome::Continue)
    }

    /// A blob the phases exchange, decoded.
    async fn load_json<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
        what: &str,
    ) -> Result<T, AgentError> {
        let bytes = self
            .store
            .get_blob(&key.to_owned())
            .await?
            .ok_or_else(|| AgentError::Unsupported(format!("{what} {key} is missing")))?;
        Ok(serde_json::from_slice(&bytes).map_err(|e| StoreError::Corrupt(e.to_string()))?)
    }

    /// The attempt protocol's retraction (R22.4, design.md § Durable execution): a status
    /// update whose `task/v1` metadata names the attempt renderers must drop, committed
    /// before the retry begins. The record is untouched.
    ///
    /// # Errors
    ///
    /// The store.
    pub async fn retract(&self, task: &TaskId, attempt: Attempt) -> Result<(), AgentError> {
        let record = self.record(task).await?;
        let mut update = record.status_update();
        update.metadata.get_or_insert_with(HashMap::new).insert(
            TASK_EXT.to_owned(),
            serde_json::json!({ "retracted": [attempt] }),
        );
        let event = StoredEvent {
            id: StoredEvent::event_id(&record.id, attempt, "retract", 0),
            seq: 0,
            at: now(&self.host),
            attempt,
            visibility: Visibility::All,
            response: a2a::StreamResponse::StatusUpdate(update),
        };
        self.store
            .commit(TaskCommit::of(record).with_events([event]))
            .await?;
        Ok(())
    }

    /// Folds the store's pending inbox events into the task at a boundary (R20.4–5): a
    /// message is the next user turn, a sub-agent's update answers the pending call that
    /// waits for it; each is marked applied in one commit with the turn it produced, and
    /// an id already applied is skipped (the duplicate-signal case).
    async fn apply_pending_inbox(
        &self,
        record: &mut TaskRecord,
        history: &mut Vec<Turn>,
        cx: &HookCx,
    ) -> Result<(), AgentError> {
        let pending = self.store.pending_inbox(&record.id).await?;
        let mut applied: Vec<String> = Vec::new();
        for event in pending {
            if record.applied_events.contains(&event.id) || applied.contains(&event.id) {
                applied.push(event.id.clone());
                continue;
            }
            let kind = crate::app_service::inbox::classify(&event.kind);
            let delivery = crate::app_service::inbox::Delivery {
                event: crate::app_service::inbox::EventRef {
                    id: event.id.clone(),
                    target: event.target.clone(),
                },
                kind,
            };
            let outcome = self
                .dispatcher
                .run::<crate::app_service::inbox::InboxDeliver, _, _>(
                    cx,
                    delivery,
                    |d| async move {
                        Ok(crate::app_service::inbox::Applied {
                            event: d.event.id,
                            kind: d.kind,
                        })
                    },
                )
                .await;
            match outcome {
                Ok(_) => {}
                Err(HarnessError::Vetoed(_)) => {
                    applied.push(event.id.clone()); // a vetoed delivery is consumed, never retried
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
            match &event.kind {
                a2a::StreamResponse::Message(m) => {
                    let text = message_text(m);
                    // While a participant is awaited, a message answers the oldest ask.
                    let open_ask = record.pending_calls.iter().position(|p| matches!(p.resume_on, ResumeOn::Ask { .. }));
                    if let Some(i) = open_ask {
                        let call = record.pending_calls.remove(i);
                        history.push(Turn::new(HistoryEntry::ToolResult { call_id: call.call_id, text }));
                    } else {
                        history.push(Turn::new(HistoryEntry::User { text }));
                        history.extend(media_turns(m));
                    }
                }
                a2a::StreamResponse::StatusUpdate(u) => {
                    if u.status.state.is_terminal()
                        && let Some(i) = record.pending_calls.iter().position(|p| matches!(&p.resume_on, ResumeOn::SubTask { task } if *task == u.task_id))
                    {
                        let call = record.pending_calls.remove(i);
                        let text = u.status.message.as_ref().and_then(|m| m.text()).unwrap_or_default().to_owned();
                        history.push(Turn::new(HistoryEntry::ToolResult { call_id: call.call_id, text: serde_json::json!({ "task_id": u.task_id, "state": format!("{:?}", u.status.state).to_lowercase(), "text": text }).to_string() }));
                    }
                }
                a2a::StreamResponse::ArtifactUpdate(a) => history.push(Turn::new(HistoryEntry::SubAgentReply { task: a.task_id.clone(), text: serde_json::to_string(&a.artifact).unwrap_or_default() })),
                a2a::StreamResponse::Task(_) => {}
            }
            applied.push(event.id.clone());
        }
        if applied.is_empty() {
            return Ok(());
        }
        record.applied_events.extend(applied.iter().cloned());
        let mut commit = TaskCommit::of(record.clone());
        commit.pending_calls.clone_from(&record.pending_calls);
        commit.inbox_applied = applied;
        commit.blobs.push(Self::history_blob(&record.id, history));
        self.store.commit(commit).await?;
        *record = self.record(&record.id.clone()).await?;
        Ok(())
    }

    /// The stop path (R21.8, R22.6): no further writes after the terminal commit.
    async fn stop(
        &self,
        record: TaskRecord,
        history: Vec<Turn>,
        attempt: Attempt,
        signal: Signal,
    ) -> Result<IterationOutcome, AgentError> {
        let (transition, outcome) = match signal {
            Signal::Cancel(_) => (Transition::Cancel, LoopOutcome::Canceled),
            Signal::Complete { .. } => (Transition::Complete, LoopOutcome::Completed),
        };
        let record = self.change(record, transition).await?;
        let mut commit =
            TaskCommit::of(record.clone()).with_events([status_event(&record, attempt, 9)]);
        commit.blobs.push(Self::history_blob(&record.id, &history));
        self.store.commit(commit).await?;
        self.finish_sub_tasks(&record).await?;
        Ok(IterationOutcome::Stopped { outcome })
    }

    /// Applies the sub-task policy when `parent` reached a terminal state (R12.4).
    async fn finish_sub_tasks(&self, parent: &TaskRecord) -> Result<(), AgentError> {
        if self.sub_task_policy != SubTaskPolicy::Cancel {
            return Ok(());
        }
        for sub in &parent.sub_tasks {
            let Some(record) = self.store.get_task(&sub.task).await? else {
                continue;
            };
            if record.is_terminal() {
                continue;
            }
            if self.mailbox.is_running(&record.id) {
                self.mailbox.signal(
                    &record.id,
                    Signal::Cancel(CancelReason::ParentTerminal {
                        parent: parent.id.clone(),
                    }),
                );
            } else {
                let record = self.change(record, Transition::Cancel).await?;
                self.store
                    .commit(TaskCommit::of(record.clone()).with_events([status_event(
                        &record,
                        Attempt {
                            iteration: record.iteration,
                            n: 1,
                        },
                        9,
                    )]))
                    .await?;
            }
        }
        Ok(())
    }

    /// Runs the loop for `task` to its next waiting point, inside `task.run` (R11.2,
    /// R11.4).
    ///
    /// # Errors
    ///
    /// An infrastructure failure; everything the loop decides is a [`LoopOutcome`].
    pub async fn run(&self, task: &TaskId) -> Result<LoopOutcome, AgentError> {
        if !self.mailbox.start(task) {
            return Err(AgentError::Unsupported(format!(
                "task {task}: already running"
            )));
        }
        let record = self.record(task).await;
        let result = match record {
            Ok(record) => {
                let cx = self.cx(&record);
                self.dispatcher
                    .run::<TaskRun, _, _>(&cx, task.clone(), |task| async move {
                        Box::pin(self.drive(&task))
                            .await
                            .map_err(|e| HarnessError::Internal(e.to_string()))
                    })
                    .await
                    .map_err(AgentError::from)
            }
            Err(e) => Err(e),
        };
        self.mailbox.finish(task);
        result
    }

    async fn drive(&self, task: &TaskId) -> Result<LoopOutcome, AgentError> {
        let started = Instant::now();
        loop {
            let mut record = self.record(task).await?;
            if record.is_terminal() {
                return Ok(outcome_of(&record));
            }
            if record.state == a2a::TaskState::InputRequired {
                // A waiting task wakes only for a message the inbox holds for it (a
                // forwarded signal, R23.1); otherwise it keeps waiting.
                let has_message = self.store.pending_inbox(task).await?.iter().any(|e| {
                    !record.applied_events.contains(&e.id)
                        && matches!(e.kind, a2a::StreamResponse::Message(_))
                });
                if !has_message {
                    return Ok(outcome_of(&record));
                }
                record = self.change(record, Transition::Resume).await?;
                let cx = self.cx(&record);
                let mut history = self.load_history(task).await?;
                self.apply_pending_inbox(&mut record, &mut history, &cx)
                    .await?;
                if record.state != a2a::TaskState::Working {
                    // The boundary commit carried the resume; make sure it landed.
                    let mut commit = TaskCommit::of(record.clone());
                    commit.blobs.push(Self::history_blob(task, &history));
                    self.store.commit(commit).await?;
                }
                continue;
            }
            let n = record.iteration + 1;
            let limit = if n > self.limits.max_iterations {
                Some(Limit::MaxIterations)
            } else if self
                .limits
                .wall_clock
                .is_some_and(|w| started.elapsed() > w)
            {
                Some(Limit::WallClock)
            } else {
                None
            };
            if let Some(limit) = limit {
                return self.limit(record, limit).await;
            }
            match self.run_iteration(task, n).await? {
                IterationOutcome::Continue => {}
                IterationOutcome::Delivered => return Ok(outcome_of(&self.record(task).await?)),
                IterationOutcome::Suspended { ask } => {
                    return Ok(LoopOutcome::AwaitingParticipant { ask });
                }
                IterationOutcome::Stopped { outcome } => return Ok(outcome),
            }
        }
    }

    /// Fails `record` with a typed error on its status message (R9.6): the text is the
    /// reason, the `kind` rides on the task extension's metadata.
    async fn fail_task(
        &self,
        mut record: TaskRecord,
        attempt: Attempt,
        kind: &str,
        reason: String,
    ) -> Result<IterationOutcome, AgentError> {
        let mut message = a2a::Message::new(a2a::Role::Agent, vec![a2a::Part::text(&reason)]);
        message.task_id = Some(record.id.clone());
        message.context_id = Some(record.context_id.clone());
        message.metadata = Some(HashMap::from([(
            TASK_EXT.to_owned(),
            serde_json::json!({ "error": { "kind": kind, "message": reason } }),
        )]));
        record.status_message = Some(message);
        let record = self.change(record, Transition::Fail).await?;
        self.store
            .commit(TaskCommit::of(record.clone()).with_events([status_event(&record, attempt, 1)]))
            .await?;
        Ok(IterationOutcome::Stopped {
            outcome: LoopOutcome::Failed(reason),
        })
    }

    /// Fails a task that is not running, after durable execution gave up on it (R9.6,
    /// R22.6): a task never stays `working` with nobody driving it. A terminal or
    /// waiting task is returned unchanged.
    ///
    /// # Errors
    ///
    /// A store failure.
    pub async fn fail(&self, task: &TaskId, reason: &str) -> Result<a2a::Task, AgentError> {
        let record = self.record(task).await?;
        if record.state != a2a::TaskState::Working || self.mailbox.is_running(task) {
            return Ok(record.to_a2a(Vec::new()));
        }
        let attempt = Attempt {
            iteration: record.iteration,
            n: 1,
        };
        self.fail_task(record, attempt, "infrastructure", reason.to_owned())
            .await?;
        Ok(self.record(task).await?.to_a2a(Vec::new()))
    }

    async fn limit(&self, record: TaskRecord, limit: Limit) -> Result<LoopOutcome, AgentError> {
        let attempt = Attempt {
            iteration: record.iteration,
            n: 1,
        };
        let transition = match self.on_limit {
            OnLimit::InputRequired => Transition::Limit {
                reason: limit.reason().to_owned(),
            },
            OnLimit::Fail => Transition::Fail,
        };
        let record = self.change(record, transition).await?;
        self.store
            .commit(TaskCommit::of(record.clone()).with_events([status_event(&record, attempt, 8)]))
            .await?;
        if record.is_terminal() {
            self.finish_sub_tasks(&record).await?;
        }
        Ok(LoopOutcome::LimitReached { limit })
    }

    /// The `complete` transition by `by` (R21.7–8): a running task stops at its next
    /// boundary, a waiting one completes now.
    ///
    /// # Errors
    ///
    /// An illegal move or a store failure.
    pub async fn complete(
        &self,
        task: &TaskId,
        by: &ParticipantId,
    ) -> Result<a2a::Task, AgentError> {
        let record = self.record(task).await?;
        if self.mailbox.is_running(task) {
            self.mailbox
                .signal(task, Signal::Complete { by: by.clone() });
            return Ok(record.to_a2a(Vec::new()));
        }
        let record = self.change(record, Transition::Complete).await?;
        self.store
            .commit(TaskCommit::of(record.clone()).with_events([status_event(
                &record,
                Attempt {
                    iteration: record.iteration,
                    n: 1,
                },
                9,
            )]))
            .await?;
        self.finish_sub_tasks(&record).await?;
        Ok(record.to_a2a(Vec::new()))
    }

    /// `CancelTask` (R21.8).
    ///
    /// # Errors
    ///
    /// An illegal move or a store failure.
    pub async fn cancel(
        &self,
        task: &TaskId,
        reason: CancelReason,
    ) -> Result<a2a::Task, AgentError> {
        let record = self.record(task).await?;
        if record.state == a2a::TaskState::Canceled {
            return Ok(record.to_a2a(Vec::new())); // idempotent: the SDK cancels in two steps
        }
        if self.mailbox.is_running(task) {
            self.mailbox.signal(task, Signal::Cancel(reason));
            return Ok(record.to_a2a(Vec::new()));
        }
        let record = self.change(record, Transition::Cancel).await?;
        self.store
            .commit(TaskCommit::of(record.clone()).with_events([status_event(
                &record,
                Attempt {
                    iteration: record.iteration,
                    n: 1,
                },
                9,
            )]))
            .await?;
        self.finish_sub_tasks(&record).await?;
        Ok(record.to_a2a(Vec::new()))
    }
}

fn outcome_of(record: &TaskRecord) -> LoopOutcome {
    match record.state {
        a2a::TaskState::Completed => LoopOutcome::Completed,
        a2a::TaskState::Canceled => LoopOutcome::Canceled,
        a2a::TaskState::Failed | a2a::TaskState::Rejected => {
            LoopOutcome::Failed(format!("{:?}", record.state))
        }
        a2a::TaskState::InputRequired => match &record.sub_state {
            Some(crate::harness::core_loop::task::SubState::AwaitingParticipant { ask }) => {
                LoopOutcome::AwaitingParticipant { ask: ask.clone() }
            }
            _ => LoopOutcome::AwaitingTurn,
        },
        _ => LoopOutcome::AwaitingTurn,
    }
}

impl Entity for LocalAgent {
    fn kind(&self) -> EntityKind {
        EntityKind::Agent
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Agent for LocalAgent {
        fn card(&self) -> &a2a::AgentCard {
            &self.card
        }

        fn schemas(&self) -> AgentSchemas {
            AgentSchemas::default()
        }

        /// Opens or continues the task, runs the loop to its next waiting point, and
        /// streams the events the call produced from the store's log.
        fn send_message(&self, request: AgentRequest) -> HarnessStream<'_, Result<a2a::StreamResponse, AgentError>> {
            let fut = async move {
                let task = request.call.task_id.clone();
                let caller = request.call.caller.clone();
                let viewer = caller.clone();
                let from = self.store.events(&task, 1, usize::MAX, &viewer).await.map_or(1, |e| e.len() as u64 + 1);
                let result: Result<Vec<a2a::StreamResponse>, AgentError> = async {
                    let (_, run) = self.open_task(&request).await?;
                    if run {
                        self.run(&task).await?;
                    }
                    // The viewer sees the channels it is a member of (R18.5), including
                    // one the run just opened.
                    let channels = crate::harness::channels::Channel::memberships(self.store.as_ref(), &task, &caller.id).await?;
                    let viewer = caller.clone().in_channels(channels);
                    let events = self.store.events(&task, from, usize::MAX, &viewer).await?;
                    Ok(events.into_iter().map(|e| e.response).collect())
                }
                .await;
                match result {
                    Ok(events) => events.into_iter().map(Ok).collect::<Vec<_>>(),
                    Err(e) => vec![Err(e)],
                }
            };
            boxed_stream(futures::StreamExt::flat_map(futures::stream::once(fut), futures::stream::iter))
        }

        async fn list_tasks(&self, context_id: &ContextId, call: CallContext) -> Result<Vec<a2a::Task>, AgentError> {
            let query = TaskQuery {
                viewer: call.caller,
                states: None,
                context_id: Some(context_id.clone()),
                open_only: false,
                after: None,
                ..TaskQuery::everyone()
            };
            let mut query = query;
            query.everyone = false;
            let page = self.store.list_tasks(query).await?;
            Ok(page.items.into_iter().map(|r| r.to_a2a(Vec::new())).collect())
        }

        async fn cancel_task(&self, task_id: &TaskId, call: CallContext) -> Result<a2a::Task, AgentError> {
            self.cancel(task_id, CancelReason::Participant { by: call.caller.id }).await
        }
    }
}

/// A sub-task's plan link helper for the loop and the channels (R12.3).
#[must_use]
pub fn sub_task_ref(record: &TaskRecord) -> crate::harness::core_loop::task::TaskRef {
    crate::harness::core_loop::task::TaskRef {
        task: record.id.clone(),
        context_id: Some(record.context_id.clone()),
    }
}

/// Plan handle re-export for callers that persist plans with the loop.
pub type TaskPlan = Plan;
/// State handle re-export for callers that persist state with the loop.
pub type TaskState = AgentState;

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        Agent, AgentRequest, CallContext, Limit, LocalAgent, LocalAgentParts, LoopLimits,
        LoopOutcome, OnLimit,
    };
    use crate::app_service::durable::CancelReason;
    use crate::app_service::inbox::{DeliveryKind, EventTarget, InboxEvent, InboxStatus};
    use crate::assets::PromptRegistry;
    use crate::config::Config;
    use crate::harness::commands::{ModelRouter, ModelSelection};
    use crate::harness::core_loop::persistence::MemoryStore;
    use crate::harness::core_loop::task::{Caller, Role, SubState, TASK_EXT};
    use crate::harness::hooks::HookDispatcher;
    use crate::harness::models::llm::testing::RecordedLlm;
    use crate::harness::models::llm::{Llm, ModelEvent, StopReason};
    use crate::harness::tools::tests::Fixture;
    use crate::harness::tools::{Idempotency, Tool, ToolCall};
    use crate::host::Host;
    use crate::registry::Registry;
    use a2a::TaskState;
    use futures::StreamExt as _;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    pub(crate) fn text_turn(text: &str) -> Vec<ModelEvent> {
        vec![
            ModelEvent::TextDelta { text: text.into() },
            ModelEvent::Stop {
                reason: StopReason::EndTurn,
            },
        ]
    }

    pub(crate) fn tool_turn(id: &str, name: &str, args: serde_json::Value) -> Vec<ModelEvent> {
        vec![
            ModelEvent::TextDelta {
                text: format!("calling {name}"),
            },
            ModelEvent::ToolCall {
                call: ToolCall {
                    id: id.into(),
                    name: name.into(),
                    arguments: args,
                },
            },
            ModelEvent::Stop {
                reason: StopReason::ToolUse,
            },
        ]
    }

    pub(crate) struct Rig {
        pub(crate) agent: Arc<LocalAgent>,
        pub(crate) llm: Arc<RecordedLlm>,
        pub(crate) write: Arc<Fixture>,
    }

    pub(crate) fn rig(turns: Vec<Vec<ModelEvent>>) -> Rig {
        rig_with(Arc::new(RecordedLlm::new("default", turns)))
    }

    pub(crate) fn rig_with(llm: Arc<RecordedLlm>) -> Rig {
        let registry = Arc::new(Registry::new());
        let as_llm: Arc<dyn Llm> = llm.clone();
        registry.register(as_llm).unwrap();
        let write = Arc::new(Fixture::new(
            "write",
            Idempotency::SideEffecting { accepts_key: true },
        ));
        for tool in [
            Arc::new(Fixture::new("echo", Idempotency::Idempotent)),
            Arc::new(Fixture::new("ask", Idempotency::Idempotent)),
            Arc::clone(&write),
        ] {
            let tool: Arc<dyn Tool> = tool;
            registry.register(tool).unwrap();
        }
        let router = Arc::new(ModelRouter::new(
            Arc::clone(&registry),
            ModelSelection {
                model: "default".into(),
                effort: None,
            },
        ));
        let agent = LocalAgent::new(LocalAgentParts {
            config: Config::default(),
            registry,
            dispatcher: Arc::new(HookDispatcher::new()),
            host: Arc::new(Host::testing()),
            store: Arc::new(MemoryStore::new()),
            prompts: Arc::new(PromptRegistry::builtin()),
            router,
        });
        Rig {
            agent: Arc::new(agent),
            llm,
            write,
        }
    }

    pub(crate) fn request(task: &str, text: &str, long_running: bool) -> AgentRequest {
        let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text(text)]);
        message.task_id = Some(task.into());
        message.context_id = Some("ctx-1".into());
        if long_running {
            message.metadata = Some(HashMap::from([(
                TASK_EXT.to_owned(),
                json!({ "mode": "long-running" }),
            )]));
        }
        AgentRequest {
            message,
            call: CallContext {
                task_id: task.into(),
                context_id: "ctx-1".into(),
                stored_task: None,
                caller: Caller::human("u1", [Role::Admin]),
                service_params: std::collections::BTreeMap::default(),
            },
            metadata: None,
            extensions: vec![],
            inbox_event_id: None,
            traceparent: None,
        }
    }

    /// R9.6: a non-retryable provider error fails the task with a typed error on its
    /// status, instead of leaving it `working` for durable execution to retry forever.
    #[tokio::test]
    async fn a_non_retryable_model_error_fails_the_task_with_a_typed_error() {
        let r = rig_with(Arc::new(RecordedLlm::failing_then(
            "default",
            1,
            false,
            text_turn("never reached"),
        )));
        r.agent
            .open_task(&request("t-fail", "hi", false))
            .await
            .unwrap();
        let outcome = r
            .agent
            .run(&"t-fail".into())
            .await
            .expect("the loop decided");
        assert!(matches!(outcome, LoopOutcome::Failed(_)), "{outcome:?}");
        let record = r
            .agent
            .store()
            .get_task(&"t-fail".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.state, a2a::TaskState::Failed);
        let status = record.status_message.expect("a status message");
        assert!(
            status.text().is_some_and(|t| !t.is_empty()),
            "the reason is the text"
        );
        assert_eq!(
            status.metadata.as_ref().unwrap()[TASK_EXT]["error"]["kind"],
            "model"
        );
        // Retryable failures stay infrastructure failures: the durable layer retries.
        let r = rig_with(Arc::new(RecordedLlm::failing_then(
            "default",
            1,
            true,
            text_turn("later"),
        )));
        r.agent
            .open_task(&request("t-retry", "hi", false))
            .await
            .unwrap();
        assert!(r.agent.run(&"t-retry".into()).await.is_err());
        let record = r
            .agent
            .store()
            .get_task(&"t-retry".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.state, a2a::TaskState::Working);
        assert_eq!(
            r.agent
                .fail(&"t-retry".into(), "gave up")
                .await
                .unwrap()
                .status
                .state,
            a2a::TaskState::Failed,
            "durable execution fails it on the record when it gives up"
        );
    }

    async fn send(agent: &LocalAgent, req: AgentRequest) -> Vec<a2a::StreamResponse> {
        agent
            .send_message(req)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|r| r.expect("event"))
            .collect()
    }

    /// Scenario: A long-running task stays in input-required across three turns
    ///     Given a long-running task
    ///     When three messages arrive in turn, each answered without tool calls
    ///     Then after each the task is input-required, awaiting the next turn, with the same id
    ///     And the third model call saw the whole prior conversation
    #[tokio::test]
    async fn a_long_running_task_stays_in_input_required_across_three_turns() {
        let r = rig(vec![text_turn("one"), text_turn("two"), text_turn("three")]);
        for (i, text) in ["first", "second", "third"].iter().enumerate() {
            let events = send(&r.agent, request("t1", text, true)).await;
            assert!(
                events.iter().any(
                    |e| matches!(e, a2a::StreamResponse::Message(m) if m.role == a2a::Role::Agent)
                ),
                "turn {i}: the reply is streamed"
            );
            let record = r
                .agent
                .store()
                .get_task(&"t1".into())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(record.state, TaskState::InputRequired, "turn {i}");
            assert_eq!(record.sub_state, Some(SubState::AwaitingNextTurn));
            assert_eq!(record.iteration as usize, i + 1);
        }
        let third = &r.llm.requests()[2];
        let all: String = third
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .map(|p| match p {
                crate::harness::models::llm::ContentPart::Text { text } => text.clone(),
                _ => String::new(),
            })
            .collect();
        assert!(
            all.contains("first")
                && all.contains("one")
                && all.contains("second")
                && all.contains("two")
                && all.contains("third"),
            "{all}"
        );
        // A one-shot task closes on delivery (R21.1).
        let r = rig(vec![text_turn("done")]);
        send(&r.agent, request("t2", "hi", false)).await;
        assert_eq!(
            r.agent
                .store()
                .get_task(&"t2".into())
                .await
                .unwrap()
                .unwrap()
                .state,
            TaskState::Completed
        );
        // A follow-up on a terminal task is refused (R21.6).
        let err = r
            .agent
            .send_message(request("t2", "more", false))
            .collect::<Vec<_>>()
            .await
            .remove(0)
            .unwrap_err();
        assert!(matches!(err, super::AgentError::Unsupported(_)), "{err}");
    }

    /// Scenario: Completing a working task stops in-flight activities and ends completed
    #[tokio::test]
    async fn completing_a_working_task_stops_in_flight_activities_and_ends_completed() {
        let r = rig(vec![
            tool_turn("c1", "echo", json!({ "text": "slow", "ms": 400 })),
            text_turn("never"),
        ]);
        let agent = Arc::clone(&r.agent);
        let handle = tokio::spawn(async move { send(&agent, request("t3", "go", true)).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(r.agent.mailbox().is_running(&"t3".into()));
        let token = r.agent.mailbox().token(&"t3".into());
        let wire = r
            .agent
            .complete(&"t3".into(), &"u1".to_owned())
            .await
            .unwrap();
        assert_eq!(
            wire.status.state,
            TaskState::Working,
            "the stop lands at the boundary"
        );
        assert!(token.is_cancelled(), "in-flight activities are told");
        handle.await.unwrap();
        let record = r
            .agent
            .store()
            .get_task(&"t3".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.state, TaskState::Completed);
        assert_eq!(r.llm.attempts(), 1, "no second model call after complete");
        assert!(!r.agent.mailbox().is_running(&"t3".into()));
    }

    /// Scenario: A task whose parent completes is canceled by the default policy
    #[tokio::test]
    async fn a_task_whose_parent_completes_is_canceled_by_the_default_policy() {
        let r = rig(vec![text_turn("parent"), text_turn("child")]);
        send(&r.agent, request("parent", "plan it", true)).await;
        let mut child = request("child", "do a step", true);
        child
            .message
            .metadata
            .as_mut()
            .unwrap()
            .get_mut(TASK_EXT)
            .unwrap()["parent"] = json!("parent");
        send(&r.agent, child).await;
        let parent = r
            .agent
            .store()
            .get_task(&"parent".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            parent
                .sub_tasks
                .iter()
                .map(|s| s.task.as_str())
                .collect::<Vec<_>>(),
            ["child"]
        );
        assert_eq!(
            r.agent
                .store()
                .get_task(&"child".into())
                .await
                .unwrap()
                .unwrap()
                .parent_tasks[0]
                .task,
            "parent"
        );
        let wire = r
            .agent
            .complete(&"parent".into(), &"u1".to_owned())
            .await
            .unwrap();
        assert_eq!(wire.status.state, TaskState::Completed);
        assert_eq!(
            r.agent
                .store()
                .get_task(&"child".into())
                .await
                .unwrap()
                .unwrap()
                .state,
            TaskState::Canceled
        );
    }

    /// Scenario: An inbox event for a running task is applied at the next loop boundary
    #[tokio::test]
    async fn an_inbox_event_for_a_running_task_is_applied_at_the_next_loop_boundary() {
        let r = rig(vec![
            tool_turn("c1", "echo", json!({ "text": "slow", "ms": 300 })),
            text_turn("done"),
        ]);
        let agent = Arc::clone(&r.agent);
        let handle = tokio::spawn(async move { send(&agent, request("t4", "start", true)).await });
        tokio::time::sleep(Duration::from_millis(80)).await;
        let note = a2a::Message::new(
            a2a::Role::User,
            vec![a2a::Part::text("also mention the weather")],
        );
        r.agent.mailbox().deliver(
            &"t4".into(),
            InboxEvent {
                id: "in-1".into(),
                target: EventTarget::Task("t4".into()),
                kind: a2a::StreamResponse::Message(note),
                status: InboxStatus::Received,
                sender: Caller::human("u1", [Role::Admin]),
                received_at: chrono::Utc::now(),
                delivery: DeliveryKind::Interrupt,
                extensions: vec![],
                traceparent: None,
            },
        );
        handle.await.unwrap();
        let requests = r.llm.requests();
        assert_eq!(requests.len(), 2);
        let second: String = requests[1]
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .map(|p| match p {
                crate::harness::models::llm::ContentPart::Text { text } => text.clone(),
                _ => String::new(),
            })
            .collect();
        assert!(
            second.contains("[user-message]\nalso mention the weather"),
            "{second}"
        );
        assert!(!requests[0].messages.iter().any(|m| m.parts.iter().any(|p| matches!(p, crate::harness::models::llm::ContentPart::Text { text } if text.contains("weather")))));
    }

    #[tokio::test]
    async fn limits_stop_the_loop_with_a_typed_outcome_and_suspending_tools_wait() {
        let r = rig(vec![
            tool_turn("c1", "echo", json!({ "text": "a" })),
            tool_turn("c2", "echo", json!({ "text": "b" })),
            tool_turn("c3", "echo", json!({ "text": "c" })),
        ]);
        let agent = Arc::try_unwrap(r.agent).ok().unwrap().with_limits(
            LoopLimits {
                max_iterations: 2,
                max_total_tokens: None,
                wall_clock: None,
            },
            OnLimit::InputRequired,
        );
        let (_, run) = agent.open_task(&request("t5", "loop", true)).await.unwrap();
        assert!(run);
        assert_eq!(
            agent.run(&"t5".into()).await.unwrap(),
            LoopOutcome::LimitReached {
                limit: Limit::MaxIterations
            }
        );
        let record = agent.store().get_task(&"t5".into()).await.unwrap().unwrap();
        assert_eq!(record.state, TaskState::InputRequired);
        assert_eq!(
            record.sub_state,
            Some(SubState::Degraded {
                reason: "loop.limit.iterations".into()
            })
        );
        assert_eq!(r.llm.attempts(), 2);

        let r = rig(vec![tool_turn("c1", "ask", json!({ "text": "q-1" }))]);
        let events = send(&r.agent, request("t6", "ask them", true)).await;
        let record = r
            .agent
            .store()
            .get_task(&"t6".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.sub_state,
            Some(SubState::AwaitingParticipant { ask: "q-1".into() })
        );
        assert_eq!(record.pending_calls[0].tool, "ask");
        assert!(events.iter().any(|e| matches!(e, a2a::StreamResponse::StatusUpdate(u) if u.status.state == TaskState::InputRequired)));
        let wire = r
            .agent
            .cancel(&"t6".into(), CancelReason::Participant { by: "u1".into() })
            .await
            .unwrap();
        assert_eq!(wire.status.state, TaskState::Canceled);
        assert_eq!(r.write.writes.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}

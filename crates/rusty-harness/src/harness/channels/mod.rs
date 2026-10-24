//! Channels: the `Channel` entity and the participant tools (R18).
//!
//! A channel is a task in `input-required` plus the `channels/v1` extension; the reply is
//! a follow-up message (design.md § Participants and channels). Two tools make the
//! model's choice explicit (R18.4): [`ParticipantAskTool`] (`participant.ask`) suspends
//! the loop with a question addressed to members, and [`ParticipantDelegateTool`]
//! (`participant.delegate`) creates a task for a participant, linked as a sub-task
//! (R18.3). The harness owns membership (R18.5): a question addressed to a subset of the
//! participants is visible to members only, and a reply from a non-member is refused.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::harness::core_loop::persistence::{
    BlobWrite, PersistenceStore, StoredEvent, TaskCommit, Visibility,
};
use crate::harness::core_loop::task::{
    Attempt, EXT_PREFIX, Participant, ParticipantId, ParticipantKind, PendingCall, ResumeOn, Role,
    TaskDraft, TaskId, TaskMode, TaskRecord, TaskRef,
};
use crate::harness::hooks::{HookPoint, Operation};
use crate::harness::tools::{Idempotency, Tool, ToolCx, ToolDescriptor, ToolError, ToolOutput};
use crate::harness_async_trait;
use crate::registry::{Entity, EntityId, EntityKind};

/// The `channels/v1` extension URI.
pub const CHANNELS_EXT: &str =
    "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/channels/v1";
/// The A2UI extension URI, for a question carried as an A2UI payload.
pub const A2UI_EXT: &str = "https://a2ui.org/a2a-extension/a2ui/v0.9.1";
/// The media type of an A2UI data part.
pub const A2UI_MEDIA_TYPE: &str = "application/a2ui+json";

/// A channel: the harness owns its membership (R18.5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Channel {
    /// The id (also the ask id the pending call waits on).
    pub id: String,
    /// The task it belongs to.
    pub task: TaskId,
    /// Who may read and write on it.
    pub members: BTreeSet<ParticipantId>,
    /// Who opened it (the agent, for an ask).
    pub created_by: ParticipantId,
    /// Whether it still accepts messages.
    pub open: bool,
}

impl Channel {
    /// The store key.
    #[must_use]
    pub fn key(id: &str) -> String {
        format!("channel/{id}")
    }

    /// Whether `participant` may read and write on it.
    #[must_use]
    pub fn admits(&self, participant: &str) -> bool {
        self.members.contains(participant)
    }

    /// Loads the channel `id`, if the store has it.
    ///
    /// # Errors
    ///
    /// A store failure or a record that does not parse.
    pub async fn load(
        store: &dyn PersistenceStore,
        id: &str,
    ) -> Result<Option<Self>, crate::harness::core_loop::persistence::StoreError> {
        let Some(bytes) = store.get_blob(&Self::key(id)).await? else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| crate::harness::core_loop::persistence::StoreError::Corrupt(e.to_string()))
    }

    /// The store key of a task's channel index.
    #[must_use]
    pub fn index_key(task: &str) -> String {
        format!("channels/{task}")
    }

    /// The ids of the task's channels.
    ///
    /// # Errors
    ///
    /// A store failure.
    pub async fn ids_of(
        store: &dyn PersistenceStore,
        task: &str,
    ) -> Result<Vec<String>, crate::harness::core_loop::persistence::StoreError> {
        Ok(match store.get_blob(&Self::index_key(task)).await? {
            Some(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            None => Vec::new(),
        })
    }

    /// The channels of `task` that admit `participant`: what a viewer's `channels` set
    /// holds when the harness resolves it (the service layer does the same from the
    /// authenticated caller).
    ///
    /// # Errors
    ///
    /// A store failure.
    pub async fn memberships(
        store: &dyn PersistenceStore,
        task: &str,
        participant: &str,
    ) -> Result<Vec<String>, crate::harness::core_loop::persistence::StoreError> {
        let mut out = Vec::new();
        for id in Self::ids_of(store, task).await? {
            if let Some(c) = Self::load(store, &id).await?
                && c.admits(participant)
            {
                out.push(id);
            }
        }
        Ok(out)
    }

    /// Persists the channel and adds it to its task's index.
    ///
    /// # Errors
    ///
    /// A store failure.
    pub async fn save(
        &self,
        store: &dyn PersistenceStore,
    ) -> Result<(), crate::harness::core_loop::persistence::StoreError> {
        store
            .put_blob(&Self::key(&self.id), self.blob().bytes)
            .await?;
        let mut ids = Self::ids_of(store, &self.task).await?;
        if !ids.contains(&self.id) {
            ids.push(self.id.clone());
            store
                .put_blob(
                    &Self::index_key(&self.task),
                    bytes::Bytes::from(serde_json::to_vec(&ids).unwrap_or_default()),
                )
                .await?;
        }
        Ok(())
    }

    /// The blob write that persists it.
    #[must_use]
    pub fn blob(&self) -> BlobWrite {
        BlobWrite {
            key: Self::key(&self.id),
            bytes: bytes::Bytes::from(serde_json::to_vec(self).unwrap_or_default()),
        }
    }
}

/// What kind of question an ask carries.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum QuestionKind {
    /// Plain text.
    #[default]
    Text,
    /// An A2UI payload (`application/a2ui+json`).
    A2ui,
}

/// The `channels/v1` payload: on the status message of an ask, on a reply message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChannelsMeta {
    /// The channel.
    pub channel: String,
    /// The ask (the pending call's resume key).
    pub ask: String,
    /// Whom the question addresses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<ParticipantId>,
    /// What the question is.
    #[serde(default)]
    pub question_kind: QuestionKind,
    /// On a reply: the ask it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

impl ChannelsMeta {
    /// The payload on `message`, if any.
    #[must_use]
    pub fn of(message: &a2a::Message) -> Option<Self> {
        let value = message.metadata.as_ref()?.get(CHANNELS_EXT)?;
        serde_json::from_value(value.clone()).ok()
    }
}

/// The input of `participant.ask`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ask {
    /// The task.
    pub task: TaskId,
    /// The model's call id.
    pub call_id: String,
    /// Whom to ask.
    pub to: Vec<ParticipantId>,
    /// The question: text, or an A2UI payload.
    pub question: Value,
    /// Text or A2UI.
    pub kind: QuestionKind,
    /// A soft deadline, in seconds; a late answer still resumes the call (R18.2).
    #[serde(default)]
    pub timeout_s: Option<u64>,
}

/// The `participant.ask` operation (R18.2, R18.4): a channel adapter hook delivers the
/// question to members who are not connected over A2A (R18.6).
#[derive(Debug)]
pub struct ParticipantAsk;

impl Operation for ParticipantAsk {
    const POINT: HookPoint = HookPoint::ParticipantAsk;
    type Input = Ask;
    type Output = Channel;
    fn identity(input: &Ask) -> String {
        input.to.join(",")
    }
}

/// The input of `participant.delegate`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delegation {
    /// The current task.
    pub task: TaskId,
    /// The delegate.
    pub to: ParticipantId,
    /// The new task's name.
    pub name: String,
    /// Its goal.
    pub goal: String,
    /// How to tell it is done.
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

/// The `participant.delegate` operation (R18.3–4).
#[derive(Debug)]
pub struct ParticipantDelegate;

impl Operation for ParticipantDelegate {
    const POINT: HookPoint = HookPoint::ParticipantDelegate;
    type Input = Delegation;
    type Output = TaskRecord;
    fn identity(input: &Delegation) -> String {
        input.to.clone()
    }
}

/// The question as the status message of the waiting task, with its `channels/v1`
/// metadata.
#[must_use]
pub fn question_message(task: &TaskRecord, channel: &Channel, ask: &Ask) -> a2a::Message {
    let part = match ask.kind {
        QuestionKind::Text => a2a::Part::text(
            ask.question
                .as_str()
                .map_or_else(|| ask.question.to_string(), str::to_owned),
        ),
        QuestionKind::A2ui => {
            a2a::Part::data(ask.question.clone()).with_media_type(A2UI_MEDIA_TYPE)
        }
    };
    let mut message = a2a::Message::new(a2a::Role::Agent, vec![part]);
    message.task_id = Some(task.id.clone());
    message.context_id = Some(task.context_id.clone());
    let meta = ChannelsMeta {
        channel: channel.id.clone(),
        ask: channel.id.clone(),
        to: ask.to.clone(),
        question_kind: ask.kind,
        reply_to: None,
    };
    let mut metadata = HashMap::from([(
        CHANNELS_EXT.to_owned(),
        serde_json::to_value(meta).unwrap_or(Value::Null),
    )]);
    if ask.kind == QuestionKind::A2ui {
        message.extensions = Some(vec![A2UI_EXT.to_owned()]);
        metadata.insert(A2UI_EXT.to_owned(), Value::Object(serde_json::Map::new()));
    }
    message.metadata = Some(metadata);
    message
}

/// The question's status update for subscribers outside the channel: the state change
/// and the sub-state, the question redacted (R18.5, abuse 2).
#[must_use]
pub fn redacted_status(task: &TaskRecord, channel: &Channel) -> a2a::TaskStatusUpdateEvent {
    let mut update = task.status_update();
    if let Some(message) = update.status.message.as_mut() {
        message.parts.clear();
        if let Some(meta) = message.metadata.as_mut() {
            meta.remove(A2UI_EXT);
            meta.insert(
                CHANNELS_EXT.to_owned(),
                serde_json::json!({ "channel": channel.id, "ask": channel.id, "question_kind": "text", "redacted": true }),
            );
        }
    }
    update
}

/// `participant.ask`: suspends the loop until a member answers (R18.2).
pub struct ParticipantAskTool {
    id: EntityId,
    descriptor: ToolDescriptor,
    store: Arc<dyn PersistenceStore>,
    dispatcher: Arc<crate::harness::hooks::HookDispatcher>,
}

impl core::fmt::Debug for ParticipantAskTool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ParticipantAskTool").finish_non_exhaustive()
    }
}

/// The agent-visible name of the ask tool.
pub const ASK_TOOL: &str = "participant.ask";
/// The agent-visible name of the delegate tool.
pub const DELEGATE_TOOL: &str = "participant.delegate";

impl ParticipantAskTool {
    /// The tool over `store`, running `participant.ask` hooks on `dispatcher`.
    #[must_use]
    pub fn new(
        store: Arc<dyn PersistenceStore>,
        dispatcher: Arc<crate::harness::hooks::HookDispatcher>,
    ) -> Self {
        Self {
            id: EntityId::new(ASK_TOOL),
            descriptor: ToolDescriptor {
                name: ASK_TOOL.into(),
                description: "Ask one or more participants of this task for an input, an opinion or a judgement smaller than a task. The loop waits for the answer.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "required": ["to", "question"],
                    "additionalProperties": false,
                    "properties": {
                        "to": { "type": "array", "minItems": 1, "items": { "type": "string" } },
                        "question": {},
                        "kind": { "type": "string", "enum": ["text", "a2ui"] },
                        "timeout_s": { "type": "integer", "minimum": 0 }
                    }
                }),
                idempotency: Idempotency::SideEffecting { accepts_key: true },
            },
            store,
            dispatcher,
        }
    }
}

impl Entity for ParticipantAskTool {
    fn kind(&self) -> EntityKind {
        EntityKind::Tool
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Tool for ParticipantAskTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.descriptor
        }
        async fn call(&self, cx: &ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
            let record = self
                .store
                .get_task(&cx.task)
                .await
                .map_err(|e| ToolError::Failed { tool: ASK_TOOL.into(), reason: e.to_string() })?
                .ok_or_else(|| ToolError::Failed { tool: ASK_TOOL.into(), reason: format!("task {} not found", cx.task) })?;
            let to: Vec<ParticipantId> = input["to"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
            for p in &to {
                if !record.participants.iter().any(|x| &x.id == p) {
                    return Err(ToolError::InvalidInput { tool: ASK_TOOL.into(), path: "/to".into(), reason: format!("{p} is not a participant of the task") });
                }
            }
            let kind = if input["kind"] == "a2ui" { QuestionKind::A2ui } else { QuestionKind::Text };
            let ask = Ask {
                task: cx.task.clone(),
                call_id: cx.call_id.clone(),
                to: to.clone(),
                question: input["question"].clone(),
                kind,
                timeout_s: input["timeout_s"].as_u64(),
            };
            let channel = Channel {
                id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, cx.idempotency_key.as_bytes()).to_string(),
                task: cx.task.clone(),
                members: to.iter().cloned().collect(),
                created_by: record.participants.iter().find(|p| matches!(p.kind, ParticipantKind::Agent(_))).map_or_else(|| "harness".to_owned(), |p| p.id.clone()),
                open: true,
            };
            let mut hook_cx = crate::harness::hooks::HookCx::new(Arc::new(crate::registry::Registry::new()), Arc::clone(&cx.host));
            hook_cx.task = Some(cx.task.clone());
            let store = Arc::clone(&self.store);
            let channel = self
                .dispatcher
                .run::<ParticipantAsk, _, _>(&hook_cx, ask.clone(), move |_ask| async move {
                    channel.save(store.as_ref()).await.map_err(|e| crate::HarnessError::Internal(e.to_string()))?;
                    Ok(channel)
                })
                .await
                .map_err(|e| match e {
                    crate::HarnessError::Vetoed(v) => ToolError::Denied { tool: ASK_TOOL.into(), reason: v.reason },
                    other => ToolError::Failed { tool: ASK_TOOL.into(), reason: other.to_string() },
                })?;
            let message = question_message(&record, &channel, &ask);
            Ok(ToolOutput::Suspend {
                pending: PendingCall { call_id: cx.call_id.clone(), tool: ASK_TOOL.into(), resume_on: ResumeOn::Ask { ask: channel.id } },
                message: Some(Box::new(message)),
            })
        }
    }
}

/// `participant.delegate`: creates a task for a participant, linked as a sub-task (R18.3).
pub struct ParticipantDelegateTool {
    id: EntityId,
    descriptor: ToolDescriptor,
    store: Arc<dyn PersistenceStore>,
    dispatcher: Arc<crate::harness::hooks::HookDispatcher>,
    model: crate::harness::commands::ModelSelection,
}

impl core::fmt::Debug for ParticipantDelegateTool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ParticipantDelegateTool")
            .finish_non_exhaustive()
    }
}

impl ParticipantDelegateTool {
    /// The tool over `store`.
    #[must_use]
    pub fn new(
        store: Arc<dyn PersistenceStore>,
        dispatcher: Arc<crate::harness::hooks::HookDispatcher>,
        model: crate::harness::commands::ModelSelection,
    ) -> Self {
        Self {
            id: EntityId::new(DELEGATE_TOOL),
            descriptor: ToolDescriptor {
                name: DELEGATE_TOOL.into(),
                description: "Create a task for a participant when the work is theirs and this task needs it done. Returns the new task's id; the current task continues.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "required": ["to", "name", "goal"],
                    "additionalProperties": false,
                    "properties": {
                        "to": { "type": "string" },
                        "name": { "type": "string", "minLength": 1 },
                        "goal": { "type": "string", "minLength": 1 },
                        "acceptance_criteria": { "type": "array", "items": { "type": "string" } }
                    }
                }),
                idempotency: Idempotency::SideEffecting { accepts_key: true },
            },
            store,
            dispatcher,
            model,
        }
    }
}

impl Entity for ParticipantDelegateTool {
    fn kind(&self) -> EntityKind {
        EntityKind::Tool
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

fn status_event(task: &TaskRecord, at: DateTime<Utc>) -> StoredEvent {
    StoredEvent {
        id: StoredEvent::event_id(&task.id, Attempt::default(), "status", 0),
        seq: 0,
        at,
        attempt: Attempt::default(),
        visibility: Visibility::All,
        response: a2a::StreamResponse::StatusUpdate(task.status_update()),
    }
}

harness_async_trait! {
    impl Tool for ParticipantDelegateTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.descriptor
        }
        async fn call(&self, cx: &ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
            let failed = |e: String| ToolError::Failed { tool: DELEGATE_TOOL.into(), reason: e };
            let mut parent = self
                .store
                .get_task(&cx.task)
                .await
                .map_err(|e| failed(e.to_string()))?
                .ok_or_else(|| failed(format!("task {} not found", cx.task)))?;
            let to = input["to"].as_str().unwrap_or_default().to_owned();
            let Some(delegate) = parent.participants.iter().find(|p| p.id == to).cloned() else {
                return Err(ToolError::InvalidInput { tool: DELEGATE_TOOL.into(), path: "/to".into(), reason: format!("{to} is not a participant of the task") });
            };
            let delegation = Delegation {
                task: cx.task.clone(),
                to: to.clone(),
                name: input["name"].as_str().unwrap_or_default().to_owned(),
                goal: input["goal"].as_str().unwrap_or_default().to_owned(),
                acceptance_criteria: input["acceptance_criteria"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect(),
            };
            let reporter = parent.participants.iter().find(|p| p.has(Role::Assignee)).cloned();
            let model = self.model.clone();
            let now = cx.host.clock().map_or_else(|_| Utc::now(), |c| c.now());
            let parent_ref = TaskRef { task: parent.id.clone(), context_id: Some(parent.context_id.clone()) };
            let context_id = parent.context_id.clone();
            let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, format!("delegate/{}", cx.idempotency_key).as_bytes()).to_string();
            let mut hook_cx = crate::harness::hooks::HookCx::new(Arc::new(crate::registry::Registry::new()), Arc::clone(&cx.host));
            hook_cx.task = Some(cx.task.clone());
            let record = self
                .dispatcher
                .run::<ParticipantDelegate, _, _>(&hook_cx, delegation, move |d| async move {
                    let mut participants = vec![Participant { id: delegate.id.clone(), display: delegate.display.clone(), kind: delegate.kind.clone(), roles: BTreeSet::from([Role::Assignee]) }];
                    if let Some(mut r) = reporter {
                        r.roles = BTreeSet::from([Role::Reporter]);
                        if r.id != delegate.id {
                            participants.push(r);
                        }
                    }
                    Ok(TaskRecord::new(
                        TaskDraft {
                            id,
                            context_id,
                            name: d.name,
                            r#type: Some("delegated".into()),
                            goal: d.goal,
                            description: String::new(),
                            acceptance_criteria: d.acceptance_criteria,
                            mode: TaskMode::LongRunning,
                            participants,
                            parent_tasks: vec![parent_ref],
                            reference_task_ids: Vec::new(),
                            model,
                        },
                        now,
                    ))
                })
                .await
                .map_err(|e| match e {
                    crate::HarnessError::Vetoed(v) => ToolError::Denied { tool: DELEGATE_TOOL.into(), reason: v.reason },
                    other => failed(other.to_string()),
                })?;
            self.store.commit(TaskCommit::of(record.clone()).with_events([status_event(&record, now)])).await.map_err(|e| failed(e.to_string()))?;
            parent.link_sub_task(TaskRef { task: record.id.clone(), context_id: Some(record.context_id.clone()) });
            self.store.commit(TaskCommit::of(parent)).await.map_err(|e| failed(e.to_string()))?;
            Ok(ToolOutput::Value { value: serde_json::json!({ "task_id": record.id, "assignee": to, "state": "submitted" }) })
        }
    }
}

/// The prefix every rusty-harness extension shares, re-exported for the card builder.
pub const EXTENSION_PREFIX: &str = EXT_PREFIX;

#[cfg(test)]
mod tests {
    use super::{
        ASK_TOOL, CHANNELS_EXT, Channel, ChannelsMeta, DELEGATE_TOOL, ParticipantAskTool,
        ParticipantDelegateTool,
    };
    use crate::assets::PromptRegistry;
    use crate::config::Config;
    use crate::harness::commands::{ModelRouter, ModelSelection};
    use crate::harness::core_loop::agent::tests::{request, text_turn, tool_turn};
    use crate::harness::core_loop::agent::{Agent, AgentError, LocalAgent, LocalAgentParts};
    use crate::harness::core_loop::persistence::{MemoryStore, PersistenceStore};
    use crate::harness::core_loop::task::{Caller, Role, SubState};
    use crate::harness::hooks::HookDispatcher;
    use crate::harness::models::llm::testing::RecordedLlm;
    use crate::harness::models::llm::{ContentPart, Llm, MessageRole, ModelEvent};
    use crate::harness::tools::Tool;
    use crate::host::Host;
    use crate::registry::Registry;
    use a2a::TaskState;
    use futures::StreamExt as _;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn harness(turns: Vec<Vec<ModelEvent>>) -> (Arc<LocalAgent>, Arc<RecordedLlm>) {
        let registry = Arc::new(Registry::new());
        let llm = Arc::new(RecordedLlm::new("default", turns));
        let as_llm: Arc<dyn Llm> = llm.clone();
        registry.register(as_llm).unwrap();
        let dispatcher = Arc::new(HookDispatcher::new());
        let store: Arc<dyn PersistenceStore> = Arc::new(MemoryStore::new());
        let selection = ModelSelection {
            model: "default".into(),
            effort: None,
        };
        let ask: Arc<dyn Tool> = Arc::new(ParticipantAskTool::new(
            Arc::clone(&store),
            Arc::clone(&dispatcher),
        ));
        let delegate: Arc<dyn Tool> = Arc::new(ParticipantDelegateTool::new(
            Arc::clone(&store),
            Arc::clone(&dispatcher),
            selection.clone(),
        ));
        registry.register(ask).unwrap();
        registry.register(delegate).unwrap();
        let router = Arc::new(ModelRouter::new(Arc::clone(&registry), selection));
        let agent = Arc::new(LocalAgent::new(LocalAgentParts {
            config: Config::default(),
            registry,
            dispatcher,
            host: Arc::new(Host::testing()),
            store,
            prompts: Arc::new(PromptRegistry::builtin()),
            router,
        }));
        (agent, llm)
    }

    fn reply(
        task: &str,
        text: &str,
        channel: &str,
        caller: Caller,
    ) -> crate::harness::core_loop::agent::AgentRequest {
        let mut req = request(task, text, true);
        req.message.metadata = Some(HashMap::from([(
            CHANNELS_EXT.to_owned(),
            json!({ "channel": channel, "ask": channel }),
        )]));
        req.call.caller = caller;
        req
    }

    fn tool_messages(llm: &RecordedLlm, i: usize) -> Vec<(String, String)> {
        llm.requests()[i]
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .map(|m| {
                (
                    m.tool_call_id.clone().unwrap_or_default(),
                    match &m.parts[0] {
                        ContentPart::Text { text } => text.clone(),
                        _ => String::new(),
                    },
                )
            })
            .collect()
    }

    /// Scenario: A participant ask round-trips through a channel
    ///     Given the model asks the reporter a question through `participant.ask`
    ///     Then the task waits in input-required, awaiting the participant, the question
    ///     on its status message with channels/v1 metadata, visible to members only
    ///     When the reporter answers on the channel
    ///     Then the loop resumes and the answer is the pending call's tool result
    /// Scenario: A task watcher outside a channel sees the state change but not the question
    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn a_participant_ask_round_trips_through_a_channel() {
        let (agent, llm) = harness(vec![
            tool_turn(
                "c1",
                ASK_TOOL,
                json!({ "to": ["u1"], "question": "Deploy to prod?" }),
            ),
            text_turn("Deploying."),
        ]);
        let events = agent
            .send_message(request("t1", "ship it", true))
            .collect::<Vec<_>>()
            .await;
        let record = agent.store().get_task(&"t1".into()).await.unwrap().unwrap();
        assert_eq!(record.state, TaskState::InputRequired);
        let Some(SubState::AwaitingParticipant { ask }) = record.sub_state.clone() else {
            panic!("{:?}", record.sub_state)
        };
        let question = record
            .status_message
            .as_ref()
            .expect("the question is the status message");
        assert_eq!(question.text(), Some("Deploy to prod?"));
        let meta = ChannelsMeta::of(question).unwrap();
        assert_eq!(
            (meta.channel.as_str(), meta.to.as_slice()),
            (ask.as_str(), &["u1".to_owned()][..])
        );
        let channel = Channel::load(agent.store().as_ref(), &ask)
            .await
            .unwrap()
            .expect("owned by the harness");
        assert!(channel.admits("u1") && !channel.admits("stranger"));
        // The caller (a member) saw the full question; the final event carries it.
        let last_status = events
            .iter()
            .rev()
            .find_map(|e| match e {
                Ok(a2a::StreamResponse::StatusUpdate(u)) => Some(u.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            last_status.status.message.as_ref().and_then(|m| m.text()),
            Some("Deploy to prod?")
        );
        // A watcher outside the channel sees the state change but not the question.
        let watcher = Caller::human("w", [Role::Watcher]);
        let outside = agent
            .store()
            .events(&"t1".into(), 1, usize::MAX, &watcher)
            .await
            .unwrap();
        let seen: Vec<_> = outside
            .iter()
            .filter_map(|e| match &e.response {
                a2a::StreamResponse::StatusUpdate(u)
                    if u.status.state == TaskState::InputRequired =>
                {
                    Some(u.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].status.message.as_ref().unwrap().parts.is_empty(),
            "question redacted"
        );
        assert_eq!(
            seen[0]
                .status
                .message
                .as_ref()
                .unwrap()
                .metadata
                .as_ref()
                .unwrap()[CHANNELS_EXT]["redacted"],
            true
        );
        let member = Caller::human("u1", [Role::Reporter]).in_channels([ask.clone()]);
        let inside = agent
            .store()
            .events(&"t1".into(), 1, usize::MAX, &member)
            .await
            .unwrap();
        assert_eq!(
            inside.len(),
            outside.len() + 1,
            "members get the full update too"
        );

        // A non-member's reply on the channel is refused as not-found (abuse 2).
        let err = agent
            .send_message(reply(
                "t1",
                "yes",
                &ask,
                Caller::human("stranger", [Role::Watcher]),
            ))
            .collect::<Vec<_>>()
            .await
            .remove(0)
            .unwrap_err();
        assert!(matches!(err, AgentError::NotFound(_)), "{err}");

        // The reporter answers; the pending call resolves and the loop resumes.
        agent
            .send_message(reply(
                "t1",
                "yes, go",
                &ask,
                Caller::human("u1", [Role::Reporter, Role::Admin]),
            ))
            .collect::<Vec<_>>()
            .await;
        let record = agent.store().get_task(&"t1".into()).await.unwrap().unwrap();
        assert_eq!(record.sub_state, Some(SubState::AwaitingNextTurn));
        assert_eq!(record.pending_calls.len(), 0);
        let tools = tool_messages(&llm, 1);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].0, "c1", "the original call");
        assert!(
            tools[0].1.contains("[tool-result]\nyes, go"),
            "{}",
            tools[0].1
        );
    }

    /// Scenario: A participant answering after the tool timeout resumes the original suspended call
    #[tokio::test]
    async fn a_participant_answering_after_the_tool_timeout_resumes_the_original_suspended_call() {
        let (agent, llm) = harness(vec![
            tool_turn(
                "c7",
                ASK_TOOL,
                json!({ "to": ["u1"], "question": "Which region?", "timeout_s": 0 }),
            ),
            text_turn("eu-west-1 it is."),
        ]);
        agent
            .send_message(request("t2", "pick a region", true))
            .collect::<Vec<_>>()
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        let record = agent.store().get_task(&"t2".into()).await.unwrap().unwrap();
        assert_eq!(
            record.pending_calls[0].call_id, "c7",
            "still pending after the soft deadline: no activity waits"
        );
        // A plain follow-up while a participant is awaited answers the open ask.
        agent
            .send_message(request("t2", "eu-west-1", true))
            .collect::<Vec<_>>()
            .await;
        let tools = tool_messages(&llm, 1);
        assert_eq!(tools[0].0, "c7");
        assert!(tools[0].1.contains("eu-west-1"));
        assert_eq!(
            agent
                .store()
                .get_task(&"t2".into())
                .await
                .unwrap()
                .unwrap()
                .sub_state,
            Some(SubState::AwaitingNextTurn)
        );
    }

    #[tokio::test]
    async fn delegation_creates_a_linked_task_for_the_participant() {
        let (agent, llm) = harness(vec![
            tool_turn(
                "c1",
                DELEGATE_TOOL,
                json!({ "to": "u1", "name": "Review the PR", "goal": "approve or request changes", "acceptance_criteria": ["a review is posted"] }),
            ),
            text_turn("Asked u1 to review."),
        ]);
        agent
            .send_message(request("t3", "get the PR reviewed", true))
            .collect::<Vec<_>>()
            .await;
        let parent = agent.store().get_task(&"t3".into()).await.unwrap().unwrap();
        assert_eq!(parent.sub_tasks.len(), 1);
        let child = agent
            .store()
            .get_task(&parent.sub_tasks[0].task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(child.state, TaskState::Submitted);
        assert_eq!(child.name, "Review the PR");
        assert!(
            child
                .participants
                .iter()
                .any(|p| p.id == "u1" && p.has(Role::Assignee))
        );
        assert!(
            child
                .participants
                .iter()
                .any(|p| p.id == "rusty-harness" && p.has(Role::Reporter)),
            "{:?}",
            child.participants
        );
        assert_eq!(child.parent_tasks[0].task, "t3");
        let tools = tool_messages(&llm, 1);
        assert!(tools[0].1.contains(&child.id));
        // Unknown participants are refused as invalid input.
        let (agent, llm) = harness(vec![
            tool_turn(
                "c1",
                DELEGATE_TOOL,
                json!({ "to": "nobody", "name": "x", "goal": "y" }),
            ),
            text_turn("ok"),
        ]);
        agent
            .send_message(request("t4", "go", true))
            .collect::<Vec<_>>()
            .await;
        assert!(tool_messages(&llm, 1)[0].1.contains("invalid-input"));
    }
}

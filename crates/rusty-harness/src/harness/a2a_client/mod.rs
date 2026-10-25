//! Outbound A2A: `RemoteAgent`, `LoopbackAgent`, and the browser transport over the
//! host's `fetch` (R17, R15.3).
//!
//! A sub-agent is an [`Agent`] whose body runs elsewhere: [`RemoteAgent`] wraps an
//! [`AgentTransport`] and is registered only after its card validated (R17.1, R17.5).
//! Natively the transport is the official client crate over JSON-RPC (`official`);
//! on `wasm32` it is this crate's own JSON-RPC over the host's HTTP client
//! (`wasm_transport`, decision-003); [`LoopbackTransport`] calls the harness's own
//! [`LocalAgent`] in process, so a loopback sub-agent is a `RemoteAgent` whose card is
//! the harness's own (R15.3–4). The model reaches every sub-agent through one tool,
//! [`AgentCallTool`] (`agent.call`), which runs `a2a.send` hooks, creates the sub-task
//! link and returns the reply as the tool result (R17.3); nothing in that path depends
//! on the sub-agent's implementation (R17.4).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use futures::StreamExt as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::HarnessError;
use crate::harness::core_loop::agent::{
    Agent, AgentError, AgentRequest, AgentSchemas, CallContext, LocalAgent,
};
use crate::harness::core_loop::persistence::{PersistenceStore, TaskCommit};
use crate::harness::core_loop::task::{Caller, PendingCall, ResumeOn, TASK_EXT, TaskId, TaskRef};
use crate::harness::hooks::{HookCx, HookDispatcher, HookPoint, Operation};
use crate::harness::tools::{Idempotency, Tool, ToolCx, ToolDescriptor, ToolError, ToolOutput};
use crate::harness_async_trait;
use crate::registry::{Entity, EntityId, EntityKind, Registry};
use crate::runtime::{HarnessStream, boxed_stream};

#[cfg(feature = "rt-tokio")]
pub mod official;
#[cfg(target_arch = "wasm32")]
pub mod wasm_transport;

/// The events of one streamed call.
pub type ResponseStream = HarnessStream<'static, Result<a2a::StreamResponse, a2a::A2AError>>;

harness_async_trait! {
    /// How a [`RemoteAgent`] reaches its peer: the official client natively, the host's
    /// HTTP client in the browser, the local agent for loopback. Every method is an A2A
    /// operation on A2A types.
    pub trait AgentTransport: crate::runtime::MaybeSend + crate::runtime::MaybeSync {
        /// `SendStreamingMessage`; `extensions` are the URIs to activate (`A2A-Extensions`).
        async fn send_streaming_message(&self, request: a2a::SendMessageRequest, extensions: Vec<String>) -> Result<ResponseStream, a2a::A2AError>;
        /// `GetTask`.
        async fn get_task(&self, request: a2a::GetTaskRequest) -> Result<a2a::Task, a2a::A2AError>;
        /// `CancelTask`.
        async fn cancel_task(&self, request: a2a::CancelTaskRequest) -> Result<a2a::Task, a2a::A2AError>;
    }
}

/// Why a card was refused (R17.5).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CardError {
    /// The card could not be fetched.
    #[error("agent card at {url}: {reason}")]
    Fetch {
        /// Where.
        url: String,
        /// What happened.
        reason: String,
    },
    /// The card does not parse.
    #[error("agent card: {0}")]
    Parse(String),
    /// A field the protocol requires is missing or wrong.
    #[error("agent card {name:?}: {reason}")]
    Invalid {
        /// The card's name, when it had one.
        name: String,
        /// What is wrong.
        reason: String,
    },
}

/// Validates a card: a name and a version, at least one interface with an absolute
/// `https` URL (or `http` to loopback) and protocol version 1.x (R17.1, R17.5, S9).
///
/// # Errors
///
/// [`CardError::Invalid`] naming the first problem.
pub fn validate_card(card: &a2a::AgentCard) -> Result<(), CardError> {
    let invalid = |reason: String| CardError::Invalid {
        name: card.name.clone(),
        reason,
    };
    if card.name.trim().is_empty() {
        return Err(invalid("name is empty".into()));
    }
    if card.version.trim().is_empty() {
        return Err(invalid("version is empty".into()));
    }
    if card.supported_interfaces.is_empty() {
        return Err(invalid("no supported interface".into()));
    }
    for iface in &card.supported_interfaces {
        let url: url::Url = iface
            .url
            .parse()
            .map_err(|e| invalid(format!("interface url {}: {e}", iface.url)))?;
        let loopback = url
            .host_str()
            .is_some_and(|h| h == "localhost" || h == "127.0.0.1" || h == "::1" || h == "[::1]");
        if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
            return Err(invalid(format!(
                "interface url {}: https required off loopback",
                iface.url
            )));
        }
        if !iface.protocol_version.starts_with("1.") {
            return Err(invalid(format!(
                "protocol version {} is not 1.x",
                iface.protocol_version
            )));
        }
    }
    Ok(())
}

/// The JSON-RPC interface of a card, if any.
#[must_use]
pub fn jsonrpc_interface(card: &a2a::AgentCard) -> Option<&a2a::AgentInterface> {
    card.supported_interfaces
        .iter()
        .find(|i| i.protocol_binding.eq_ignore_ascii_case("JSONRPC"))
}

/// A sub-agent reached over A2A (R17).
pub struct RemoteAgent {
    id: EntityId,
    card: a2a::AgentCard,
    transport: Arc<dyn AgentTransport>,
}

impl core::fmt::Debug for RemoteAgent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RemoteAgent")
            .field("id", &self.id)
            .field("card", &self.card.name)
            .finish_non_exhaustive()
    }
}

impl RemoteAgent {
    /// A remote agent for a validated card.
    ///
    /// # Errors
    ///
    /// [`CardError`] when the card fails validation; the agent is not constructed, so it
    /// can never be registered or shown to the model (R17.5).
    pub fn new(
        id: EntityId,
        card: a2a::AgentCard,
        transport: Arc<dyn AgentTransport>,
    ) -> Result<Self, CardError> {
        validate_card(&card)?;
        Ok(Self {
            id,
            card,
            transport,
        })
    }

    /// One line for the sub-agent index (R17.2): name, description and skills.
    #[must_use]
    pub fn index_line(&self) -> String {
        let skills: Vec<&str> = self.card.skills.iter().map(|s| s.name.as_str()).collect();
        if skills.is_empty() {
            format!("{}: {}", self.id.id, self.card.description)
        } else {
            format!(
                "{}: {} (skills: {})",
                self.id.id,
                self.card.description,
                skills.join(", ")
            )
        }
    }

    /// The transport.
    #[must_use]
    pub fn transport(&self) -> &Arc<dyn AgentTransport> {
        &self.transport
    }
}

impl Entity for RemoteAgent {
    fn kind(&self) -> EntityKind {
        EntityKind::Agent
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Agent for RemoteAgent {
        fn card(&self) -> &a2a::AgentCard {
            &self.card
        }

        fn schemas(&self) -> AgentSchemas {
            AgentSchemas::default()
        }

        fn send_message(&self, request: AgentRequest) -> HarnessStream<'_, Result<a2a::StreamResponse, AgentError>> {
            let mut message = request.message;
            message.task_id.get_or_insert(request.call.task_id.clone());
            message.context_id.get_or_insert(request.call.context_id.clone());
            let wire = a2a::SendMessageRequest {
                message,
                configuration: None,
                metadata: request.metadata,
                tenant: None,
            };
            let extensions = request.extensions;
            let fut = async move {
                match self.transport.send_streaming_message(wire, extensions).await {
                    Ok(stream) => boxed_stream(stream.map(|r| r.map_err(|e| AgentError::Model(e.to_string())))),
                    Err(e) => boxed_stream(futures::stream::iter(vec![Err(AgentError::Unsupported(e.to_string()))])),
                }
            };
            boxed_stream(futures::stream::once(fut).flatten())
        }

        async fn cancel_task(&self, task_id: &TaskId, _call: CallContext) -> Result<a2a::Task, AgentError> {
            self.transport
                .cancel_task(a2a::CancelTaskRequest { id: task_id.clone(), metadata: None, tenant: None })
                .await
                .map_err(|e| AgentError::Unsupported(e.to_string()))
        }
    }
}

/// The in-memory transport: an A2A message to the harness's own [`LocalAgent`], the
/// message still an A2A message (S5, R15.3).
pub struct LoopbackTransport {
    local: Arc<LocalAgent>,
    caller: Caller,
}

impl core::fmt::Debug for LoopbackTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LoopbackTransport").finish_non_exhaustive()
    }
}

impl LoopbackTransport {
    /// A transport calling `local` as `caller` (the harness itself).
    #[must_use]
    pub fn new(local: Arc<LocalAgent>, caller: Caller) -> Self {
        Self { local, caller }
    }
}

harness_async_trait! {
    impl AgentTransport for LoopbackTransport {
        async fn send_streaming_message(&self, request: a2a::SendMessageRequest, extensions: Vec<String>) -> Result<ResponseStream, a2a::A2AError> {
            let task_id = request.message.task_id.clone().unwrap_or_else(a2a::new_task_id);
            let context_id = request.message.context_id.clone().unwrap_or_else(a2a::new_context_id);
            let agent_request = AgentRequest {
                message: request.message,
                call: CallContext { task_id, context_id, stored_task: None, caller: self.caller.clone(), service_params: BTreeMap::new() },
                metadata: request.metadata,
                extensions,
                inbox_event_id: None,
            };
            let local = Arc::clone(&self.local);
            let events: Vec<Result<a2a::StreamResponse, a2a::A2AError>> = local
                .send_message(agent_request)
                .map(|r| r.map_err(|e| e.to_a2a()))
                .collect()
                .await;
            Ok(boxed_stream(futures::stream::iter(events)))
        }

        async fn get_task(&self, request: a2a::GetTaskRequest) -> Result<a2a::Task, a2a::A2AError> {
            self.local
                .store()
                .a2a_task(&request.id, &self.caller)
                .await
                .map_err(|e| a2a::A2AError::internal(e.to_string()))?
                .ok_or_else(|| a2a::A2AError::task_not_found(&request.id))
        }

        async fn cancel_task(&self, request: a2a::CancelTaskRequest) -> Result<a2a::Task, a2a::A2AError> {
            self.local
                .cancel(&request.id, crate::app_service::durable::CancelReason::Participant { by: self.caller.id.clone() })
                .await
                .map_err(|e| e.to_a2a())
        }
    }
}

/// The harness as its own sub-agent: a [`RemoteAgent`] over [`LoopbackTransport`] with
/// the local agent's card (R15.3–4).
///
/// # Errors
///
/// [`CardError`] when the local card does not validate (no interface configured yet:
/// a loopback card gets a loopback interface so it does).
pub fn loopback_agent(local: &Arc<LocalAgent>) -> Result<RemoteAgent, CardError> {
    let mut card = local.card().clone();
    if card.supported_interfaces.is_empty() {
        card.supported_interfaces =
            vec![a2a::AgentInterface::new("http://127.0.0.1:0/", "JSONRPC")];
    }
    let caller = Caller::system(local.entity_id().id.to_string());
    RemoteAgent::new(
        EntityId::new(format!("{}/loopback", local.entity_id().id)),
        card,
        Arc::new(LoopbackTransport::new(Arc::clone(local), caller)),
    )
}

/// What `agent.call` sends (the `a2a.send` input).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutboundMessage {
    /// The sub-agent, by registry id.
    pub agent: String,
    /// The parent task.
    pub parent: TaskId,
    /// The message.
    pub message: a2a::Message,
    /// Whether the caller suspends until the sub-task ends.
    pub wait: bool,
}

/// What came back (the `a2a.send` output).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentReply {
    /// The sub-agent's task id, once known.
    pub task_id: Option<TaskId>,
    /// The final state, when the stream reached one.
    pub state: Option<a2a::TaskState>,
    /// The reply text (the last agent message or status message).
    pub text: String,
    /// The artifacts.
    pub artifacts: Vec<a2a::Artifact>,
}

/// The `a2a.send` operation (sherma `agent_call`; security-relevant, R17.3): the guard
/// stage refuses an unregistered agent (abuse 5).
#[derive(Debug)]
pub struct A2aSend;

impl Operation for A2aSend {
    const POINT: HookPoint = HookPoint::A2aSend;
    type Input = OutboundMessage;
    type Output = AgentReply;
    fn identity(input: &OutboundMessage) -> String {
        input.agent.clone()
    }
    fn attributes(input: &OutboundMessage, output: &AgentReply) -> Vec<(&'static str, String)> {
        vec![
            ("agent", input.agent.clone()),
            ("sub_task", output.task_id.clone().unwrap_or_default()),
        ]
    }
}

/// Folds a sub-agent's stream into a reply (R17.3).
///
/// # Errors
///
/// The first error the stream yields.
pub async fn fold_reply<S>(mut stream: S) -> Result<AgentReply, a2a::A2AError>
where
    S: futures::Stream<Item = Result<a2a::StreamResponse, a2a::A2AError>> + Unpin,
{
    let mut reply = AgentReply::default();
    while let Some(event) = stream.next().await {
        match event? {
            a2a::StreamResponse::Task(t) => {
                reply.task_id = Some(t.id.clone());
                reply.state = Some(t.status.state.clone());
                if let Some(m) = &t.status.message
                    && let Some(text) = m.text()
                {
                    text.clone_into(&mut reply.text);
                }
                if let Some(a) = t.artifacts {
                    reply.artifacts = a;
                }
            }
            a2a::StreamResponse::Message(m) => {
                if m.role == a2a::Role::Agent
                    && let Some(text) = m.text()
                {
                    text.clone_into(&mut reply.text);
                    if reply.task_id.is_none() {
                        reply.task_id.clone_from(&m.task_id);
                    }
                }
            }
            a2a::StreamResponse::StatusUpdate(u) => {
                reply.task_id.get_or_insert(u.task_id.clone());
                reply.state = Some(u.status.state.clone());
                if let Some(m) = &u.status.message
                    && let Some(text) = m.text()
                    && !text.is_empty()
                {
                    text.clone_into(&mut reply.text);
                }
            }
            a2a::StreamResponse::ArtifactUpdate(a) => {
                reply.task_id.get_or_insert(a.task_id.clone());
                reply.artifacts.push(a.artifact);
            }
        }
    }
    Ok(reply)
}

/// The agent-visible name of the sub-agent tool.
pub const AGENT_CALL_TOOL: &str = "agent.call";

/// `agent.call { agent, message, wait }` (R17.3, R18.4).
pub struct AgentCallTool {
    id: EntityId,
    descriptor: ToolDescriptor,
    registry: Arc<Registry>,
    dispatcher: Arc<HookDispatcher>,
    store: Arc<dyn PersistenceStore>,
}

impl core::fmt::Debug for AgentCallTool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AgentCallTool").finish_non_exhaustive()
    }
}

impl AgentCallTool {
    /// The tool over the registry's agents.
    #[must_use]
    pub fn new(
        registry: Arc<Registry>,
        dispatcher: Arc<HookDispatcher>,
        store: Arc<dyn PersistenceStore>,
    ) -> Self {
        Self {
            id: EntityId::new(AGENT_CALL_TOOL),
            descriptor: ToolDescriptor {
                name: AGENT_CALL_TOOL.into(),
                description: "Hand a piece of work to another agent and get its reply. `agent` is one of the listed sub-agents; `wait: true` suspends this task until the sub-agent's task ends.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "required": ["agent", "message"],
                    "additionalProperties": false,
                    "properties": {
                        "agent": { "type": "string" },
                        "message": { "type": "string", "minLength": 1 },
                        "wait": { "type": "boolean" }
                    }
                }),
                idempotency: Idempotency::SideEffecting { accepts_key: true },
            },
            registry,
            dispatcher,
            store,
        }
    }
}

impl Entity for AgentCallTool {
    fn kind(&self) -> EntityKind {
        EntityKind::Tool
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Tool for AgentCallTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.descriptor
        }
        async fn call(&self, cx: &ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
            let agent_id = input["agent"].as_str().unwrap_or_default().to_owned();
            let failed = |reason: String| ToolError::Failed { tool: AGENT_CALL_TOOL.into(), reason };
            // Boundary 3 / abuse 5: only a registered agent can be called.
            let Ok(agent) = self.registry.latest::<dyn Agent>(&agent_id) else {
                return Err(ToolError::Denied { tool: AGENT_CALL_TOOL.into(), reason: format!("agent {agent_id} is not registered") });
            };
            let parent = self
                .store
                .get_task(&cx.task)
                .await
                .map_err(|e| failed(e.to_string()))?
                .ok_or_else(|| failed(format!("task {} not found", cx.task)))?;
            let sub_task_id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, format!("agent.call/{}", cx.idempotency_key).as_bytes()).to_string();
            let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text(input["message"].as_str().unwrap_or_default())]);
            message.message_id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, format!("agent.call/msg/{}", cx.idempotency_key).as_bytes()).to_string();
            message.task_id = Some(sub_task_id.clone());
            message.context_id = Some(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, format!("agent.call/ctx/{}", cx.idempotency_key).as_bytes()).to_string());
            message.metadata = Some(HashMap::from([(TASK_EXT.to_owned(), serde_json::json!({ "mode": "one-shot", "parent": parent.id }))]));
            let outbound = OutboundMessage { agent: agent_id.clone(), parent: parent.id.clone(), message, wait: input["wait"].as_bool().unwrap_or(false) };
            let mut hook_cx = HookCx::new(Arc::clone(&self.registry), Arc::clone(&cx.host));
            hook_cx.task = Some(cx.task.clone());
            let caller = cx.caller.clone();
            let agent: Arc<dyn Agent> = agent;
            let reply = self
                .dispatcher
                .run::<A2aSend, _, _>(&hook_cx, outbound.clone(), move |out| async move {
                    let agent = agent;
                    let request = AgentRequest {
                        message: out.message.clone(),
                        call: CallContext { task_id: out.message.task_id.clone().unwrap_or_default(), context_id: out.message.context_id.clone().unwrap_or_default(), stored_task: None, caller, service_params: BTreeMap::new() },
                        metadata: None,
                        extensions: vec![TASK_EXT.to_owned()],
                        inbox_event_id: None,
                    };
                    let stream = agent.send_message(request).map(|r| r.map_err(|e| e.to_a2a()));
                    fold_reply(stream).await.map_err(|e| HarnessError::Internal(e.to_string()))
                })
                .await
                .map_err(|e| match e {
                    HarnessError::Vetoed(v) => ToolError::Denied { tool: AGENT_CALL_TOOL.into(), reason: v.reason },
                    other => failed(other.to_string()),
                })?;
            // The sub-task link on the parent (R17.3, R12.3); loopback already linked it.
            let linked = reply.task_id.clone().unwrap_or(sub_task_id);
            if let Some(mut parent) = self.store.get_task(&cx.task).await.map_err(|e| failed(e.to_string()))? {
                let before = parent.sub_tasks.len();
                parent.link_sub_task(TaskRef { task: linked.clone(), context_id: outbound.message.context_id.clone() });
                if parent.sub_tasks.len() != before {
                    self.store.commit(TaskCommit::of(parent)).await.map_err(|e| failed(e.to_string()))?;
                }
            }
            if outbound.wait && reply.state.as_ref().is_none_or(|s| !s.is_terminal()) {
                return Ok(ToolOutput::Suspend { pending: PendingCall { call_id: cx.call_id.clone(), tool: AGENT_CALL_TOOL.into(), resume_on: ResumeOn::SubTask { task: linked } }, message: None });
            }
            Ok(ToolOutput::Value {
                value: serde_json::json!({
                    "agent": agent_id,
                    "task_id": linked,
                    "state": reply.state.map(|s| format!("{s:?}").to_lowercase()),
                    "text": reply.text,
                    "artifacts": reply.artifacts,
                }),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AGENT_CALL_TOOL, AgentCallTool, AgentTransport, CardError, RemoteAgent, ResponseStream,
        loopback_agent, validate_card,
    };
    use crate::assets::PromptRegistry;
    use crate::config::Config;
    use crate::harness::commands::{ModelRouter, ModelSelection};
    use crate::harness::core_loop::agent::tests::{request, text_turn, tool_turn};
    use crate::harness::core_loop::agent::{Agent, LocalAgent, LocalAgentParts};
    use crate::harness::core_loop::persistence::MemoryStore;
    use crate::harness::hooks::HookDispatcher;
    use crate::harness::models::llm::testing::RecordedLlm;
    use crate::harness::models::llm::{Llm, ModelEvent};
    use crate::harness::tools::Tool;
    use crate::harness_async_trait;
    use crate::host::Host;
    use crate::registry::{EntityId, Registry};
    use crate::runtime::boxed_stream;
    use futures::StreamExt as _;
    use serde_json::json;
    use std::sync::Arc;

    fn card(name: &str, url: &str) -> a2a::AgentCard {
        a2a::AgentCard {
            name: name.into(),
            description: format!("{name} does things"),
            version: "1.0.0".into(),
            supported_interfaces: vec![a2a::AgentInterface::new(url, "JSONRPC")],
            capabilities: a2a::AgentCapabilities::default(),
            default_input_modes: vec!["text/plain".into()],
            default_output_modes: vec!["text/plain".into()],
            skills: vec![],
            provider: None,
            documentation_url: None,
            icon_url: None,
            security_schemes: None,
            security_requirements: None,
            signatures: None,
        }
    }

    /// A remote peer scripted to answer every message with one completed task.
    struct Scripted(String);
    harness_async_trait! {
        impl AgentTransport for Scripted {
            async fn send_streaming_message(&self, request: a2a::SendMessageRequest, extensions: Vec<String>) -> Result<ResponseStream, a2a::A2AError> {
                assert!(extensions.iter().any(|e| e.ends_with("/task/v1")), "the harness activates its extensions");
                let task_id = request.message.task_id.clone().unwrap();
                let context_id = request.message.context_id.clone().unwrap();
                let mut submitted = a2a::Task { id: task_id.clone(), context_id: context_id.clone(), status: a2a::TaskStatus { state: a2a::TaskState::Submitted, message: None, timestamp: None }, artifacts: None, history: None, metadata: None };
                let mut reply = a2a::Message::new(a2a::Role::Agent, vec![a2a::Part::text(self.0.clone())]);
                reply.task_id = Some(task_id.clone());
                submitted.status.state = a2a::TaskState::Submitted;
                let done = a2a::TaskStatusUpdateEvent { task_id, context_id, status: a2a::TaskStatus { state: a2a::TaskState::Completed, message: Some(reply.clone()), timestamp: None }, metadata: None };
                Ok(boxed_stream(futures::stream::iter(vec![Ok(a2a::StreamResponse::Task(submitted)), Ok(a2a::StreamResponse::Message(reply)), Ok(a2a::StreamResponse::StatusUpdate(done))])))
            }
            async fn get_task(&self, request: a2a::GetTaskRequest) -> Result<a2a::Task, a2a::A2AError> {
                Err(a2a::A2AError::task_not_found(&request.id))
            }
            async fn cancel_task(&self, request: a2a::CancelTaskRequest) -> Result<a2a::Task, a2a::A2AError> {
                Err(a2a::A2AError::task_not_cancelable(&request.id))
            }
        }
    }

    fn harness(turns: Vec<Vec<ModelEvent>>) -> (Arc<LocalAgent>, Arc<Registry>, Arc<RecordedLlm>) {
        let registry = Arc::new(Registry::new());
        let llm = Arc::new(RecordedLlm::new("default", turns));
        let as_llm: Arc<dyn Llm> = llm.clone();
        registry.register(as_llm).unwrap();
        let dispatcher = Arc::new(HookDispatcher::new());
        let store: Arc<dyn crate::harness::core_loop::persistence::PersistenceStore> =
            Arc::new(MemoryStore::new());
        let tool: Arc<dyn Tool> = Arc::new(AgentCallTool::new(
            Arc::clone(&registry),
            Arc::clone(&dispatcher),
            Arc::clone(&store),
        ));
        registry.register(tool).unwrap();
        let router = Arc::new(ModelRouter::new(
            Arc::clone(&registry),
            ModelSelection {
                model: "default".into(),
                effort: None,
            },
        ));
        let agent = Arc::new(LocalAgent::new(LocalAgentParts {
            config: Config::default(),
            registry: Arc::clone(&registry),
            dispatcher,
            host: Arc::new(Host::testing()),
            store,
            prompts: Arc::new(PromptRegistry::builtin()),
            router,
        }));
        (agent, registry, llm)
    }

    fn tool_result_text(llm: &RecordedLlm, request_index: usize) -> serde_json::Value {
        let r = &llm.requests()[request_index];
        let tool = r
            .messages
            .iter()
            .find(|m| m.role == crate::harness::models::llm::MessageRole::Tool)
            .expect("a tool result");
        let text = match &tool.parts[0] {
            crate::harness::models::llm::ContentPart::Text { text } => text.clone(),
            _ => panic!(),
        };
        let inner = text
            .trim_start_matches("[tool-result]\n")
            .trim_end_matches("\n[/tool-result]");
        serde_json::from_str(inner).expect("json tool result")
    }

    /// Scenario: A sub-agent whose card fails validation is never shown to the model
    #[tokio::test]
    async fn a_sub_agent_whose_card_fails_validation_is_never_shown_to_the_model() {
        let plain = card("plain", "http://agents.example/a2a");
        assert!(
            matches!(validate_card(&plain), Err(CardError::Invalid { ref reason, .. }) if reason.contains("https"))
        );
        let mut old = card("old", "https://agents.example/a2a");
        old.supported_interfaces[0].protocol_version = "0.3".into();
        assert!(validate_card(&old).is_err());
        let mut nameless = card("x", "https://agents.example/a2a");
        nameless.name = " ".into();
        assert!(validate_card(&nameless).is_err());
        assert!(validate_card(&card("ok", "https://agents.example/a2a")).is_ok());
        assert!(validate_card(&card("local", "http://localhost:7474/")).is_ok());

        let (agent, registry, llm) = harness(vec![text_turn("hi")]);
        assert!(
            RemoteAgent::new(
                EntityId::new("plain"),
                plain,
                Arc::new(Scripted("never".into()))
            )
            .is_err()
        );
        assert_eq!(registry.list::<dyn Agent>().len(), 0, "nothing registered");
        let _ = agent
            .send_message(request("t1", "hello", false))
            .collect::<Vec<_>>()
            .await;
        assert!(
            !llm.requests()[0]
                .instructions
                .render()
                .contains("Other agents you may hand work to"),
            "no sub-agent index without sub-agents"
        );
    }

    /// Scenario: A loopback sub-agent result is indistinguishable from a remote one
    ///     Given a remote sub-agent and the harness's own loopback agent
    ///     When the model calls each through `agent.call`
    ///     Then the tool results have the same shape, the reply text and the final state,
    ///     And each sub-task is linked on the parent
    #[tokio::test]
    async fn a_loopback_sub_agent_result_is_indistinguishable_from_a_remote_one() {
        // Remote: the peer answers out of process.
        let (agent, registry, llm) = harness(vec![
            tool_turn(
                "c1",
                AGENT_CALL_TOOL,
                json!({ "agent": "peer", "message": "sub work" }),
            ),
            text_turn("done"),
        ]);
        let peer: Arc<dyn Agent> = Arc::new(
            RemoteAgent::new(
                EntityId::new("peer"),
                card("peer", "https://peer.example/a2a"),
                Arc::new(Scripted("sub result".into())),
            )
            .unwrap(),
        );
        registry.register(peer).unwrap();
        agent
            .send_message(request("parent-r", "go", true))
            .collect::<Vec<_>>()
            .await;
        let remote = tool_result_text(&llm, 1);
        assert!(
            llm.requests()[0]
                .instructions
                .render()
                .contains("- peer: peer does things"),
            "listed at the sub-agent index"
        );
        let parent = agent
            .store()
            .get_task(&"parent-r".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(parent.sub_tasks.len(), 1);
        assert_eq!(
            parent.sub_tasks[0].task,
            remote["task_id"].as_str().unwrap()
        );

        // Loopback: the harness is its own sub-agent; the sub-task runs the same loop.
        let (agent, registry, llm) = harness(vec![
            tool_turn(
                "c1",
                AGENT_CALL_TOOL,
                json!({ "agent": "rusty-harness/loopback", "message": "sub work" }),
            ),
            text_turn("sub result"),
            text_turn("done"),
        ]);
        let loopback: Arc<dyn Agent> = Arc::new(loopback_agent(&agent).unwrap());
        registry.register(loopback).unwrap();
        agent
            .send_message(request("parent-l", "go", true))
            .collect::<Vec<_>>()
            .await;
        let local = tool_result_text(&llm, 2);
        let parent = agent
            .store()
            .get_task(&"parent-l".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(parent.sub_tasks.len(), 1);
        let sub = agent
            .store()
            .get_task(&parent.sub_tasks[0].task)
            .await
            .unwrap()
            .expect("the loopback sub-task is a real task");
        assert_eq!(sub.parent_tasks[0].task, "parent-l");
        assert_eq!(sub.state, a2a::TaskState::Completed, "one-shot, as briefed");
        assert_eq!(
            sub.participants
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["rusty-harness", "rusty-harness"],
            "the harness as reporter and assignee"
        );

        for (name, result) in [("remote", &remote), ("loopback", &local)] {
            assert_eq!(result["text"], "sub result", "{name}");
            assert_eq!(result["state"], "completed", "{name}");
            assert_eq!(result["artifacts"], json!([]), "{name}");
            assert!(result["task_id"].is_string(), "{name}");
        }
        assert_eq!(
            remote.as_object().unwrap().keys().collect::<Vec<_>>(),
            local.as_object().unwrap().keys().collect::<Vec<_>>(),
            "same shape"
        );
    }

    #[tokio::test]
    async fn abuse_agent_call_to_an_unregistered_agent_is_refused() {
        let (agent, _registry, llm) = harness(vec![
            tool_turn(
                "c1",
                AGENT_CALL_TOOL,
                json!({ "agent": "ghost", "message": "pwn" }),
            ),
            text_turn("ok"),
        ]);
        agent
            .send_message(request("t9", "go", true))
            .collect::<Vec<_>>()
            .await;
        let result = tool_result_text(&llm, 1);
        assert_eq!(result["error"]["kind"], "denied", "{result}");
        assert!(
            result["error"]["reason"]
                .as_str()
                .unwrap()
                .contains("not registered")
        );
        let parent = agent.store().get_task(&"t9".into()).await.unwrap().unwrap();
        assert_eq!(parent.sub_tasks.len(), 0, "no sub-task, no message sent");
        assert_eq!(
            agent
                .store()
                .list_tasks(
                    crate::harness::core_loop::persistence::TaskQuery::for_viewer(
                        crate::harness::core_loop::task::Caller::human("u1", [])
                    )
                )
                .await
                .unwrap()
                .items
                .len(),
            1
        );
    }
}

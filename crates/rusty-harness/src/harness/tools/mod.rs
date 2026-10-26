//! Tools: the `Tool` trait, the executor, MCP servers, the memory and sandbox seams (R8).
//!
//! A tool is an entity with a [`ToolDescriptor`]; the model calls it by name. The
//! [`ToolExecutor`] validates each call against the tool's JSON schema, runs
//! `tool.execute` per call (policy lives in the guard stage, [`ToolPolicy`]), executes
//! the model's calls concurrently and returns the results in issue order (R8.2), turns an
//! unknown tool, invalid input, a veto or a failure into a [`ToolResult`] error for the
//! model instead of failing the task (R8.3), and sends every result through
//! `tool.result.size` (R8.6). MCP servers register one [`McpTool`] per remote tool under
//! the server's `(id, version)` namespace (R8.1): natively over `rmcp` ([`mcp`]), in the
//! browser over the host's `McpTransport` ([`host_mcp`], Q14).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::HarnessError;
use crate::assets::{Prompt, PromptArgs, PromptRegistry};
use crate::harness::core_loop::task::{Attempt, Caller, PendingCall, Role, TaskId};
use crate::harness::hooks::remote::McpToolCaller;
use crate::harness::hooks::{
    GuardHook, GuardOutcome, HookCx, HookDispatcher, HookError, HookPoint, Operation, Veto,
};
use crate::harness::models::llm::{Llm, ModelRequest, collect};
use crate::harness_async_trait;
use crate::host::Host;
use crate::registry::{Entity, EntityId, EntityKind, Registry};

pub mod host_mcp;
#[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
pub mod mcp;
pub mod memory;
pub mod sandbox;

/// Whether a tool may be retried after a failure without harm (R22.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Idempotency {
    /// Running it twice has the same effect as once.
    Idempotent,
    /// It has side effects; `accepts_key` says whether an idempotency key makes a retry safe.
    SideEffecting {
        /// The tool deduplicates on the key the harness passes.
        accepts_key: bool,
    },
}

/// What the model is told about a tool, and what the executor validates a call against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// The name the model calls; unique in a request.
    pub name: String,
    /// What the tool does, for the model.
    pub description: String,
    /// The JSON Schema of the arguments.
    pub parameters: serde_json::Value,
    /// Retry safety.
    pub idempotency: Idempotency,
}

/// One tool call the model asked for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    /// The provider's id for the call, echoed back with the result.
    pub id: String,
    /// The tool's name.
    pub name: String,
    /// The arguments, as the model produced them (validated by the executor).
    pub arguments: serde_json::Value,
}

/// A typed tool error; every variant reaches the model as a tool error, never as a task
/// failure (R8.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum ToolError {
    /// No tool has that name.
    #[error("unknown tool {tool}")]
    Unknown {
        /// The name the model used.
        tool: String,
    },
    /// The arguments do not satisfy the tool's schema.
    #[error("tool {tool}: invalid input at {path}: {reason}")]
    InvalidInput {
        /// The tool.
        tool: String,
        /// A JSON pointer to the offending value.
        path: String,
        /// What was violated.
        reason: String,
    },
    /// A hook or the policy said no (abuse 4).
    #[error("tool {tool}: denied ({reason})")]
    Denied {
        /// The tool.
        tool: String,
        /// The catalog key of the reason.
        reason: String,
    },
    /// The tool ran and failed.
    #[error("tool {tool}: {reason}")]
    Failed {
        /// The tool.
        tool: String,
        /// What it said.
        reason: String,
    },
    /// The server or the transport failed.
    #[error("tool transport: {reason}")]
    Transport {
        /// What happened.
        reason: String,
    },
    /// The call was cancelled.
    #[error("tool {tool}: cancelled")]
    Cancelled {
        /// The tool.
        tool: String,
    },
}

/// A cancellation flag an executing tool observes (design.md § Tools; heartbeating tools
/// poll it between steps).
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token nobody has cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels every holder.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// What a tool sees besides its arguments.
#[derive(Clone)]
pub struct ToolCx {
    /// The task.
    pub task: TaskId,
    /// The model's call id.
    pub call_id: String,
    /// `(task, attempt, call_id)`: the key a side-effecting tool deduplicates on.
    pub idempotency_key: String,
    /// On whose behalf the tool runs.
    pub caller: Caller,
    /// The platform.
    pub host: Arc<Host>,
    /// Cancellation.
    pub cancel: CancelToken,
}

impl core::fmt::Debug for ToolCx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolCx")
            .field("task", &self.task)
            .field("call_id", &self.call_id)
            .field("caller", &self.caller.id)
            .finish_non_exhaustive()
    }
}

/// What a tool returns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "output")]
pub enum ToolOutput {
    /// A result for the model.
    Value {
        /// The result, as JSON.
        value: Value,
    },
    /// A result for the model and artifacts for the task's participants: an image a tool
    /// rendered, a file it produced (R28.2). The model sees `value` and the artifacts'
    /// names; the participants receive the artifacts as `TaskArtifactUpdateEvent`s.
    Artifacts {
        /// The result, as JSON.
        value: Value,
        /// What the participants receive.
        artifacts: Vec<a2a::Artifact>,
    },
    /// The call waits for a participant or a sub-task; the loop resumes it later.
    Suspend {
        /// What it waits for.
        pending: PendingCall,
        /// The status message to show while waiting (a channel's question), if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<Box<a2a::Message>>,
    },
}

harness_async_trait! {
    /// A tool the model can call (R8).
    pub trait Tool: Entity {
        /// What the model is told.
        fn descriptor(&self) -> &ToolDescriptor;
        /// Runs it; `input` already satisfies the descriptor's schema.
        async fn call(&self, cx: &ToolCx, input: Value) -> Result<ToolOutput, ToolError>;
    }
}

/// What reaches the model for one call (R8.2, R8.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    /// The model's call id.
    pub call_id: String,
    /// The tool.
    pub name: String,
    /// The value, or the error's JSON when `error` is set.
    pub value: Value,
    /// The error, when the call did not produce a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ToolError>,
    /// Whether the oversize policy shortened `value` (R8.6).
    #[serde(default)]
    pub shortened: bool,
    /// Artifacts the call produced for the participants (R28.2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<a2a::Artifact>,
}

impl ToolResult {
    /// A successful result.
    #[must_use]
    pub fn ok(call: &ToolCall, value: Value) -> Self {
        Self {
            call_id: call.id.clone(),
            name: call.name.clone(),
            value,
            error: None,
            shortened: false,
            artifacts: Vec::new(),
        }
    }

    /// An error result; `value` carries the error as JSON so the model sees one shape.
    #[must_use]
    pub fn error(call: &ToolCall, error: ToolError) -> Self {
        Self {
            call_id: call.id.clone(),
            name: call.name.clone(),
            value: serde_json::json!({ "error": error }),
            error: Some(error),
            shortened: false,
            artifacts: Vec::new(),
        }
    }

    /// The text the context builder hands the model.
    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut text = match &self.value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        // The model learns what the participants received, not the bytes.
        for a in &self.artifacts {
            let name = a.name.as_deref().unwrap_or(&a.artifact_id);
            let media: Vec<&str> = a
                .parts
                .iter()
                .filter_map(|p| p.media_type.as_deref())
                .collect();
            let _ = write!(
                text,
                "
[artifact {name} ({}) delivered to the participants]",
                media.join(", ")
            );
        }
        text
    }
}

/// The outcome of one call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "outcome")]
pub enum Executed {
    /// A result for the model, in issue order.
    Result {
        /// The result.
        result: ToolResult,
    },
    /// The call suspended; the record keeps the pending call.
    Suspended {
        /// The pending call.
        pending: PendingCall,
        /// The status message to show while waiting, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<Box<a2a::Message>>,
    },
}

/// The input of `tool.execute`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocation {
    /// The call.
    pub call: ToolCall,
    /// The task.
    pub task: TaskId,
    /// The attempt.
    pub attempt: Attempt,
    /// On whose behalf.
    pub caller: Caller,
}

/// The `tool.execute` operation (R8.4): security-relevant, fail-closed, the guard stage
/// holds the policy. The output is the tool's own answer, errors included, so a `post`
/// hook sees failures.
#[derive(Debug)]
pub struct ToolExecute;

impl Operation for ToolExecute {
    const POINT: HookPoint = HookPoint::ToolExecute;
    type Input = ToolInvocation;
    type Output = Result<ToolOutput, ToolError>;
    fn identity(input: &ToolInvocation) -> String {
        input.call.name.clone()
    }
    fn attributes(input: &ToolInvocation, output: &Self::Output) -> Vec<(&'static str, String)> {
        vec![
            ("tool", input.call.name.clone()),
            (
                "outcome",
                match output {
                    Ok(ToolOutput::Value { .. }) => "value".into(),
                    Ok(ToolOutput::Artifacts { artifacts, .. }) => {
                        format!("value+{} artifacts", artifacts.len())
                    }
                    Ok(ToolOutput::Suspend { .. }) => "suspend".into(),
                    Err(e) => e.to_string(),
                },
            ),
        ]
    }
}

/// The `tool.result.size` operation (R8.6): the result goes in, a result that fits the
/// context comes out.
#[derive(Debug)]
pub struct ToolResultSize;

impl Operation for ToolResultSize {
    const POINT: HookPoint = HookPoint::ToolResultSize;
    type Input = ToolResult;
    type Output = ToolResult;
    fn identity(input: &ToolResult) -> String {
        input.name.clone()
    }
}

/// What to do with a result above `max_result_bytes` (R8.6).
#[derive(Clone)]
pub enum OversizePolicy {
    /// Keep the head and the tail, mark the cut.
    Truncate,
    /// One model call on `prompts/tool/summarise-result.md`; truncation when it fails.
    Summarise {
        /// The model.
        llm: Arc<dyn Llm>,
        /// Where the fragment comes from.
        prompts: Arc<PromptRegistry>,
    },
}

impl core::fmt::Debug for OversizePolicy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Truncate => "Truncate",
            Self::Summarise { .. } => "Summarise",
        })
    }
}

/// Head and tail of `text` within `limit` bytes, with a marker in between.
#[must_use]
pub fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let marker = format!("\n…[{} bytes cut]…\n", text.len() - limit);
    let budget = limit.saturating_sub(marker.len());
    let head_len = budget * 3 / 4;
    let tail_len = budget - head_len;
    let head = text.floor_char_boundary(head_len);
    let tail = text.ceil_char_boundary(text.len().saturating_sub(tail_len));
    format!("{}{marker}{}", &text[..head], &text[tail..])
}

impl OversizePolicy {
    /// Shortens `result` to `limit` bytes when it is larger.
    pub async fn apply(&self, mut result: ToolResult, limit: usize) -> ToolResult {
        let text = result.render();
        if text.len() <= limit || result.error.is_some() {
            return result;
        }
        let shortened = match self {
            Self::Truncate => truncate(&text, limit),
            Self::Summarise { llm, prompts } => {
                summarise(llm.as_ref(), prompts, &result.name, &text, limit)
                    .await
                    .unwrap_or_else(|| truncate(&text, limit))
            }
        };
        result.value = Value::String(shortened);
        result.shortened = true;
        result
    }
}

async fn summarise(
    llm: &dyn Llm,
    prompts: &PromptRegistry,
    tool: &str,
    text: &str,
    limit: usize,
) -> Option<String> {
    let fragment = prompts.get("tool/summarise-result", "en")?;
    let args = PromptArgs::new()
        .with("tool_name", tool)
        .with("limit", limit.to_string())
        .with("result", text);
    let instructions = Prompt::fragment(&fragment, &args).ok()?;
    let request = ModelRequest::new(instructions, Vec::new());
    let events = llm.stream(request).await.ok()?;
    let turn = collect(events, None).await.ok()?;
    (!turn.text.is_empty() && turn.text.len() <= limit).then_some(turn.text)
}

/// Executes the model's tool calls (design.md § Tools and MCP).
pub struct ToolExecutor {
    registry: Arc<Registry>,
    dispatcher: Arc<HookDispatcher>,
    host: Arc<Host>,
    max_result_bytes: usize,
    oversize: OversizePolicy,
}

impl core::fmt::Debug for ToolExecutor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolExecutor")
            .field("max_result_bytes", &self.max_result_bytes)
            .field("oversize", &self.oversize)
            .finish_non_exhaustive()
    }
}

impl ToolExecutor {
    /// An executor over `registry`, running hooks on `dispatcher`.
    #[must_use]
    pub fn new(registry: Arc<Registry>, dispatcher: Arc<HookDispatcher>, host: Arc<Host>) -> Self {
        Self {
            registry,
            dispatcher,
            host,
            max_result_bytes: 64 * 1024,
            oversize: OversizePolicy::Truncate,
        }
    }

    /// The size limit and the oversize policy (`[tools]`).
    #[must_use]
    pub fn with_result_limit(mut self, max_result_bytes: usize, oversize: OversizePolicy) -> Self {
        self.max_result_bytes = max_result_bytes;
        self.oversize = oversize;
        self
    }

    /// The descriptors of every registered tool, for the model request.
    #[must_use]
    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.registry
            .list::<dyn Tool>()
            .iter()
            .map(|t| t.descriptor().clone())
            .collect()
    }

    /// Runs `calls` concurrently and returns their outcomes in issue order (R8.2).
    pub async fn execute_all(
        &self,
        cx: &HookCx,
        task: &TaskId,
        attempt: Attempt,
        caller: &Caller,
        cancel: &CancelToken,
        calls: Vec<ToolCall>,
    ) -> Vec<Executed> {
        futures::future::join_all(
            calls
                .into_iter()
                .map(|call| self.execute_one(cx, task, attempt, caller, cancel, call)),
        )
        .await
    }

    /// Runs one call through validation, `tool.execute` and `tool.result.size`.
    pub async fn execute_one(
        &self,
        cx: &HookCx,
        task: &TaskId,
        attempt: Attempt,
        caller: &Caller,
        cancel: &CancelToken,
        call: ToolCall,
    ) -> Executed {
        let Ok(tool) = self.registry.latest::<dyn Tool>(&call.name) else {
            return self
                .sized(
                    cx,
                    ToolResult::error(
                        &call,
                        ToolError::Unknown {
                            tool: call.name.clone(),
                        },
                    ),
                )
                .await;
        };
        if let Err((path, reason)) = crate::harness::core_loop::state::check_against(
            &tool.descriptor().parameters,
            &call.arguments,
        ) {
            let error = ToolError::InvalidInput {
                tool: call.name.clone(),
                path,
                reason,
            };
            return self.sized(cx, ToolResult::error(&call, error)).await;
        }
        let tool_cx = ToolCx {
            task: task.clone(),
            call_id: call.id.clone(),
            // Per iteration and call, never per attempt: a retried call must reuse the key
            // its earlier attempt used (design.md § Durable execution, R17.4).
            idempotency_key: format!("{task}/{}/{}", attempt.iteration, call.id),
            caller: caller.clone(),
            host: Arc::clone(&self.host),
            cancel: cancel.clone(),
        };
        let invocation = ToolInvocation {
            call: call.clone(),
            task: task.clone(),
            attempt,
            caller: caller.clone(),
        };
        let outcome = self
            .dispatcher
            .run::<ToolExecute, _, _>(cx, invocation, move |inv| async move {
                Ok(tool.call(&tool_cx, inv.call.arguments).await)
            })
            .await;
        match outcome {
            Ok(Ok(ToolOutput::Value { value })) => {
                self.sized(cx, ToolResult::ok(&call, value)).await
            }
            Ok(Ok(ToolOutput::Artifacts { value, artifacts })) => {
                let mut result = ToolResult::ok(&call, value);
                result.artifacts = artifacts;
                self.sized(cx, result).await
            }
            Ok(Ok(ToolOutput::Suspend { pending, message })) => {
                Executed::Suspended { pending, message }
            }
            Ok(Err(error)) => self.sized(cx, ToolResult::error(&call, error)).await,
            Err(HarnessError::Vetoed(veto)) => {
                let error = ToolError::Denied {
                    tool: call.name.clone(),
                    reason: veto.reason,
                };
                self.sized(cx, ToolResult::error(&call, error)).await
            }
            Err(other) => {
                let error = ToolError::Failed {
                    tool: call.name.clone(),
                    reason: other.to_string(),
                };
                self.sized(cx, ToolResult::error(&call, error)).await
            }
        }
    }

    async fn sized(&self, cx: &HookCx, result: ToolResult) -> Executed {
        let limit = self.max_result_bytes;
        let policy = self.oversize.clone();
        let fallback = result.clone();
        let result = self
            .dispatcher
            .run::<ToolResultSize, _, _>(cx, result, move |r| async move {
                Ok(policy.apply(r, limit).await)
            })
            .await
            .unwrap_or(fallback);
        Executed::Result { result }
    }
}

/// The tool policy, as a guard on `tool.execute` (abuse 4): admins and assignees may
/// call every tool unless an allowlist names theirs; every other caller may call
/// idempotent tools, and side-effecting ones only when its role's allowlist names them.
#[derive(Debug, Default, Clone)]
pub struct ToolPolicy {
    /// Per role, the tool names allowed; `*` allows all. A role without an entry gets
    /// the defaults above.
    pub allow: BTreeMap<Role, Vec<String>>,
}

impl ToolPolicy {
    fn listed(&self, role: Role, tool: &str) -> Option<bool> {
        self.allow
            .get(&role)
            .map(|names| names.iter().any(|n| n == "*" || n == tool))
    }

    /// Whether `caller` may call `tool`.
    #[must_use]
    pub fn permits(&self, caller: &Caller, tool: &ToolDescriptor) -> bool {
        let side_effecting = !matches!(tool.idempotency, Idempotency::Idempotent);
        caller
            .roles
            .iter()
            .any(|role| match self.listed(*role, &tool.name) {
                Some(listed) => listed,
                None => matches!(role, Role::Admin | Role::Assignee) || !side_effecting,
            })
            || (caller.roles.is_empty() && !side_effecting && self.allow.is_empty())
    }
}

harness_async_trait! {
    impl GuardHook<ToolExecute> for ToolPolicy {
        async fn guard(&self, cx: &HookCx, input: &ToolInvocation) -> Result<GuardOutcome, HookError> {
            let Ok(tool) = cx.registry.latest::<dyn Tool>(&input.call.name) else {
                return Ok(GuardOutcome::Allow); // the executor answers unknown tools itself
            };
            if self.permits(&input.caller, tool.descriptor()) {
                Ok(GuardOutcome::Allow)
            } else {
                Ok(GuardOutcome::Deny(Veto {
                    reason: crate::assets::keys::tool::DENIED.as_str().to_owned(),
                    detail: Some(input.call.name.clone()),
                    by: EntityId::new("tools.policy"),
                }))
            }
        }
    }
}

/// A tool as an MCP server lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpToolInfo {
    /// The name on the server.
    pub name: String,
    /// Its description.
    pub description: String,
    /// Its input schema.
    pub input_schema: Value,
    /// Whether the server marks it read-only or idempotent.
    pub idempotent: bool,
}

/// What an MCP tool call returned.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpCallOutcome {
    /// The structured content when the server gave one, else the text blocks joined.
    pub content: Value,
    /// The server's `isError`.
    pub is_error: bool,
}

harness_async_trait! {
    /// An open MCP session: `rmcp` natively, the host's channel in the browser (Q14).
    pub trait McpSession: Send + Sync {
        /// `tools/list`, every page.
        async fn list_tools(&self) -> Result<Vec<McpToolInfo>, ToolError>;
        /// `tools/call`.
        async fn call_tool(&self, name: &str, arguments: Value) -> Result<McpCallOutcome, ToolError>;
    }
}

/// A connected MCP server (R8.1): an entity under the plugin's `(id, version)`.
pub struct McpServer {
    id: EntityId,
    session: Arc<dyn McpSession>,
}

impl core::fmt::Debug for McpServer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("McpServer")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Entity for McpServer {
    fn kind(&self) -> EntityKind {
        EntityKind::Tool
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

impl McpServer {
    /// A server over an open session.
    #[must_use]
    pub fn new(id: EntityId, session: Arc<dyn McpSession>) -> Self {
        Self { id, session }
    }

    /// Connects to `spec` (a plugin's `mcp.json` entry, substituted) with what the
    /// platform offers: `rmcp` natively with the `mcp` feature, else the host's
    /// `McpTransport`.
    ///
    /// # Errors
    ///
    /// [`ToolError::Transport`] when neither is available or the connection fails.
    pub async fn connect(
        id: EntityId,
        spec: &crate::harness::plugins::McpServerSpec,
        host: &Host,
    ) -> Result<Self, ToolError> {
        #[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
        {
            if host.mcp().is_err() {
                let session = mcp::RmcpSession::connect(spec).await?;
                return Ok(Self::new(id, Arc::new(session)));
            }
        }
        let transport = host.mcp().map_err(|e| ToolError::Transport {
            reason: e.to_string(),
        })?;
        let session =
            host_mcp::ChannelSession::connect(transport.as_ref(), host_mcp::target_of(spec)?)
                .await?;
        Ok(Self::new(id, Arc::new(session)))
    }

    /// The session.
    #[must_use]
    pub fn session(&self) -> &Arc<dyn McpSession> {
        &self.session
    }

    /// Lists the server's tools and registers each as `<server>/<tool>` under the
    /// server's version (R8.1).
    ///
    /// # Errors
    ///
    /// A listing failure, or a registry conflict.
    pub async fn register_tools(&self, registry: &Registry) -> Result<Vec<EntityId>, ToolError> {
        let mut ids = Vec::new();
        for info in self.session.list_tools().await? {
            let name = format!("{}/{}", self.id.id, info.name);
            let id = match &self.id.version {
                Some(v) => EntityId::versioned(name.clone(), v.clone()),
                None => EntityId::new(name.clone()),
            };
            let tool: Arc<dyn Tool> = Arc::new(McpTool {
                id: id.clone(),
                remote_name: info.name,
                descriptor: ToolDescriptor {
                    name,
                    description: info.description,
                    parameters: info.input_schema,
                    idempotency: if info.idempotent {
                        Idempotency::Idempotent
                    } else {
                        Idempotency::SideEffecting { accepts_key: false }
                    },
                },
                session: Arc::clone(&self.session),
            });
            registry.register(tool).map_err(|e| ToolError::Transport {
                reason: e.to_string(),
            })?;
            ids.push(id);
        }
        Ok(ids)
    }
}

impl McpToolCaller for McpServer {
    fn call_tool(
        &self,
        tool: &str,
        arguments: Value,
    ) -> crate::runtime::HarnessFuture<'_, Result<Value, HookError>> {
        let tool = tool.to_owned();
        Box::pin(async move {
            let outcome = self
                .session
                .call_tool(&tool, arguments)
                .await
                .map_err(|e| HookError::Remote(e.to_string()))?;
            if outcome.is_error {
                return Err(HookError::Remote(outcome.content.to_string()));
            }
            Ok(outcome.content)
        })
    }
}

/// One tool of an MCP server.
pub struct McpTool {
    id: EntityId,
    remote_name: String,
    descriptor: ToolDescriptor,
    session: Arc<dyn McpSession>,
}

impl core::fmt::Debug for McpTool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("McpTool")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Entity for McpTool {
    fn kind(&self) -> EntityKind {
        EntityKind::Tool
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Tool for McpTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.descriptor
        }
        async fn call(&self, cx: &ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
            if cx.cancel.is_cancelled() {
                return Err(ToolError::Cancelled { tool: self.descriptor.name.clone() });
            }
            let outcome = self.session.call_tool(&self.remote_name, input).await?;
            if outcome.is_error {
                return Err(ToolError::Failed {
                    tool: self.descriptor.name.clone(),
                    reason: match &outcome.content {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    },
                });
            }
            Ok(ToolOutput::Value { value: outcome.content })
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        CancelToken, Executed, Idempotency, OversizePolicy, Tool, ToolCall, ToolCx, ToolDescriptor,
        ToolError, ToolExecute, ToolExecutor, ToolOutput, ToolPolicy, ToolResult, truncate,
    };
    use crate::harness::core_loop::task::{Attempt, Caller, PendingCall, ResumeOn, Role};
    use crate::harness::hooks::{
        Hook, HookCx, HookDispatcher, HookError, HookOrder, PreHook, PreOutcome,
    };
    use crate::harness_async_trait;
    use crate::host::Host;
    use crate::registry::{Entity, EntityId, EntityKind, Registry};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    /// A fixture tool: echoes, sleeps, writes (counted) or suspends.
    pub(crate) struct Fixture {
        id: EntityId,
        descriptor: ToolDescriptor,
        pub(crate) writes: AtomicU32,
    }

    impl Fixture {
        pub(crate) fn new(name: &str, idempotency: Idempotency) -> Self {
            Self {
                id: EntityId::new(name),
                descriptor: ToolDescriptor {
                    name: name.into(),
                    description: format!("the {name} fixture"),
                    parameters: json!({ "type": "object", "required": ["text"], "properties": { "text": { "type": "string" }, "ms": { "type": "integer", "minimum": 0 }, "repeat": { "type": "integer" } }, "additionalProperties": false }),
                    idempotency,
                },
                writes: AtomicU32::new(0),
            }
        }
    }

    impl Entity for Fixture {
        fn kind(&self) -> EntityKind {
            EntityKind::Tool
        }
        fn entity_id(&self) -> &EntityId {
            &self.id
        }
    }

    harness_async_trait! {
        impl Tool for Fixture {
            fn descriptor(&self) -> &ToolDescriptor {
                &self.descriptor
            }
            async fn call(&self, _cx: &ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
                let text = input["text"].as_str().unwrap_or("").to_owned();
                if let Some(ms) = input["ms"].as_u64() {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                }
                match self.descriptor.name.as_str() {
                    "write" => {
                        self.writes.fetch_add(1, Ordering::SeqCst);
                        Ok(ToolOutput::Value { value: json!({ "written": text }) })
                    }
                    "ask" => Ok(ToolOutput::Suspend { pending: PendingCall { call_id: "pending".into(), tool: "ask".into(), resume_on: ResumeOn::Ask { ask: text } }, message: None }),
                    "fail" => Err(ToolError::Failed { tool: "fail".into(), reason: text }),
                    _ => {
                        let repeat = usize::try_from(input["repeat"].as_u64().unwrap_or(1)).unwrap();
                        Ok(ToolOutput::Value { value: Value::String(text.repeat(repeat)) })
                    }
                }
            }
        }
    }

    pub(crate) struct Rig {
        pub(crate) registry: Arc<Registry>,
        pub(crate) dispatcher: Arc<HookDispatcher>,
        pub(crate) executor: ToolExecutor,
        pub(crate) cx: HookCx,
        pub(crate) write: Arc<Fixture>,
    }

    pub(crate) fn rig() -> Rig {
        let registry = Arc::new(Registry::new());
        let write = Arc::new(Fixture::new(
            "write",
            Idempotency::SideEffecting { accepts_key: true },
        ));
        for tool in [
            Arc::new(Fixture::new("echo", Idempotency::Idempotent)),
            Arc::new(Fixture::new("ask", Idempotency::Idempotent)),
            Arc::new(Fixture::new("fail", Idempotency::Idempotent)),
            Arc::clone(&write),
        ] {
            let tool: Arc<dyn Tool> = tool;
            registry.register(tool).unwrap();
        }
        let dispatcher = Arc::new(HookDispatcher::new());
        let host = Arc::new(Host::testing());
        let cx = HookCx::new(Arc::clone(&registry), Arc::clone(&host));
        let executor = ToolExecutor::new(Arc::clone(&registry), Arc::clone(&dispatcher), host)
            .with_result_limit(64, OversizePolicy::Truncate);
        Rig {
            registry,
            dispatcher,
            executor,
            cx,
            write,
        }
    }

    fn call(id: &str, name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args,
        }
    }

    fn results(executed: &[Executed]) -> Vec<&ToolResult> {
        executed
            .iter()
            .map(|e| match e {
                Executed::Result { result } => result,
                Executed::Suspended { .. } => panic!("suspended"),
            })
            .collect()
    }

    /// Scenario: Several tool calls run concurrently and return in issue order
    #[tokio::test]
    async fn several_tool_calls_run_concurrently_and_return_in_issue_order() {
        let r = rig();
        let caller = Caller::human("u1", [Role::Admin]);
        let started = Instant::now();
        let out = r
            .executor
            .execute_all(
                &r.cx,
                &"t".into(),
                Attempt { iteration: 1, n: 1 },
                &caller,
                &CancelToken::new(),
                vec![
                    call("c1", "echo", json!({ "text": "slow", "ms": 200 })),
                    call("c2", "echo", json!({ "text": "mid", "ms": 100 })),
                    call("c3", "echo", json!({ "text": "fast" })),
                ],
            )
            .await;
        assert!(
            started.elapsed() < Duration::from_millis(290),
            "ran concurrently: {:?}",
            started.elapsed()
        );
        assert_eq!(
            results(&out)
                .iter()
                .map(|r| r.call_id.as_str())
                .collect::<Vec<_>>(),
            ["c1", "c2", "c3"]
        );
        assert_eq!(
            results(&out).iter().map(|r| r.render()).collect::<Vec<_>>(),
            ["slow", "mid", "fast"]
        );
    }

    #[tokio::test]
    async fn unknown_tools_invalid_input_failures_and_suspensions_are_typed_outcomes() {
        let r = rig();
        let caller = Caller::human("u1", [Role::Admin]);
        let out = r
            .executor
            .execute_all(
                &r.cx,
                &"t".into(),
                Attempt::default(),
                &caller,
                &CancelToken::new(),
                vec![
                    call("c1", "nope", json!({})),
                    call("c2", "echo", json!({ "text": 3 })),
                    call("c3", "fail", json!({ "text": "boom" })),
                    call("c4", "ask", json!({ "text": "q1" })),
                ],
            )
            .await;
        let Executed::Result { result } = &out[0] else {
            panic!()
        };
        assert_eq!(
            result.error,
            Some(ToolError::Unknown {
                tool: "nope".into()
            })
        );
        assert_eq!(result.value["error"]["kind"], "unknown");
        let Executed::Result { result } = &out[1] else {
            panic!()
        };
        assert!(
            matches!(&result.error, Some(ToolError::InvalidInput { path, .. }) if path == "/text"),
            "{result:?}"
        );
        let Executed::Result { result } = &out[2] else {
            panic!()
        };
        assert_eq!(
            result.error,
            Some(ToolError::Failed {
                tool: "fail".into(),
                reason: "boom".into()
            })
        );
        assert!(
            matches!(&out[3], Executed::Suspended { pending, .. } if pending.resume_on == ResumeOn::Ask { ask: "q1".into() })
        );
    }

    /// Scenario: A tool result above the size limit is truncated by the configured policy
    #[tokio::test]
    async fn a_tool_result_above_the_size_limit_is_truncated_by_the_configured_policy() {
        let r = rig();
        let caller = Caller::human("u1", [Role::Admin]);
        let out = r
            .executor
            .execute_all(
                &r.cx,
                &"t".into(),
                Attempt::default(),
                &caller,
                &CancelToken::new(),
                vec![
                    call("c1", "echo", json!({ "text": "0123456789", "repeat": 50 })),
                    call("c2", "echo", json!({ "text": "short" })),
                ],
            )
            .await;
        let big = results(&out)[0];
        assert!(big.shortened);
        let text = big.render();
        assert!(text.len() <= 64, "{}", text.len());
        assert!(
            text.starts_with("0123456789") && text.ends_with("6789") && text.contains("bytes cut"),
            "{text}"
        );
        assert!(!results(&out)[1].shortened);
        assert_eq!(truncate("héllo wörld", 100), "héllo wörld");
        assert!(truncate(&"é".repeat(100), 30).len() <= 30);
    }

    /// Scenario: A pre hook veto stops tool execution and the model is told
    #[tokio::test]
    async fn a_pre_hook_veto_stops_tool_execution_and_the_model_is_told() {
        struct NoWrites;
        harness_async_trait! {
            impl PreHook<ToolExecute> for NoWrites {
                async fn pre(&self, _cx: &HookCx, input: super::ToolInvocation) -> Result<PreOutcome<super::ToolInvocation>, HookError> {
                    if input.call.name == "write" {
                        Ok(PreOutcome::Veto(crate::harness::hooks::Veto { reason: "policy.no-writes".into(), detail: None, by: EntityId::new("no-writes") }))
                    } else {
                        Ok(PreOutcome::Continue(input))
                    }
                }
            }
        }
        let r = rig();
        r.dispatcher.register::<ToolExecute>(
            EntityId::new("no-writes"),
            Hook::Pre(Arc::new(NoWrites)),
            HookOrder::default(),
        );
        let caller = Caller::human("u1", [Role::Admin]);
        let out = r
            .executor
            .execute_all(
                &r.cx,
                &"t".into(),
                Attempt::default(),
                &caller,
                &CancelToken::new(),
                vec![call("c1", "write", json!({ "text": "x" }))],
            )
            .await;
        let result = results(&out)[0];
        assert_eq!(
            result.error,
            Some(ToolError::Denied {
                tool: "write".into(),
                reason: "policy.no-writes".into()
            })
        );
        assert_eq!(r.write.writes.load(Ordering::SeqCst), 0, "never executed");
    }

    #[tokio::test]
    async fn abuse_denied_tool_call_is_not_executed_and_model_receives_a_tool_error() {
        let r = rig();
        r.dispatcher.register::<ToolExecute>(
            EntityId::new("tools.policy"),
            Hook::Guard(Arc::new(ToolPolicy::default())),
            HookOrder::default(),
        );
        let watcher = Caller::human("w", [Role::Watcher]);
        let out = r
            .executor
            .execute_all(
                &r.cx,
                &"t".into(),
                Attempt::default(),
                &watcher,
                &CancelToken::new(),
                vec![
                    call("c1", "write", json!({ "text": "x" })),
                    call("c2", "echo", json!({ "text": "ok" })),
                ],
            )
            .await;
        assert_eq!(
            results(&out)[0].error,
            Some(ToolError::Denied {
                tool: "write".into(),
                reason: "tool.denied".into()
            })
        );
        assert_eq!(
            results(&out)[1].render(),
            "ok",
            "idempotent tools stay open to every participant"
        );
        assert_eq!(r.write.writes.load(Ordering::SeqCst), 0);
        let admin = Caller::human("a", [Role::Admin]);
        let out = r
            .executor
            .execute_all(
                &r.cx,
                &"t".into(),
                Attempt::default(),
                &admin,
                &CancelToken::new(),
                vec![call("c3", "write", json!({ "text": "x" }))],
            )
            .await;
        assert!(results(&out)[0].error.is_none());
        assert_eq!(r.write.writes.load(Ordering::SeqCst), 1);
        // An allowlist narrows admins and widens watchers.
        let mut policy = ToolPolicy::default();
        policy.allow.insert(Role::Admin, vec!["echo".into()]);
        policy.allow.insert(Role::Watcher, vec!["write".into()]);
        let write = r.registry.latest::<dyn Tool>("write").unwrap();
        assert!(!policy.permits(&admin, write.descriptor()));
        assert!(policy.permits(&watcher, write.descriptor()));
        assert!(!policy.permits(&Caller::system("agent"), write.descriptor()));
    }
}

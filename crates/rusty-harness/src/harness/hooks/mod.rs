//! Hooks on the whole lifecycle: the `HookPoint` catalogue, `Operation`, the dispatcher
//! with its guard stage, remote hooks (R4).
//!
//! Every lifecycle operation runs through [`HookDispatcher::run`] in four stages:
//! mutable `pre` hooks (may rewrite the input or veto), the **guard** stage (policy:
//! allow or deny on the final input, never mutate, always fail-closed), the `in` chain
//! (the innermost hook wraps the default implementation and may replace it, but may not
//! move the operation's target), and `post` hooks (may rewrite the output). Hooks run in
//! a deterministic order: by priority, then by declared `after` constraints, then by
//! registration order (R4.5).

use core::any::{Any, TypeId};
use core::fmt;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::HarnessError;
use crate::harness_async_trait;
use crate::host::Host;
use crate::registry::{EntityId, Registry};
use crate::runtime::{HarnessFuture, MaybeSend};

pub mod remote;

/// Every point of the catalogue (design.md § Hooks). Names are `area.operation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum HookPoint {
    /// `harness.start`.
    HarnessStart,
    /// `harness.stop`.
    HarnessStop,
    /// `request.receive`: after authentication, before the inbox.
    RequestReceive,
    /// `inbox.accept`: the durable store of an event. Fail-closed.
    InboxAccept,
    /// `inbox.deliver`: interrupt or queue (sherma `interrupt`).
    InboxDeliver,
    /// `task.create`.
    TaskCreate,
    /// `task.state_change`.
    TaskStateChange,
    /// `task.run`: the whole loop (sherma `graph_invoke`).
    TaskRun,
    /// `step.enter`.
    StepEnter,
    /// `step.execute`.
    StepExecute,
    /// `step.exit`.
    StepExit,
    /// `step.error`.
    StepError,
    /// `prompt.assemble` (R7.3).
    PromptAssemble,
    /// `context.build` (R15.5).
    ContextBuild,
    /// `context.compact` (R16.3).
    ContextCompact,
    /// `model.create` (sherma `model_create`).
    ModelCreate,
    /// `model.call` (sherma `model_call`).
    ModelCall,
    /// `response.parse`: extract tool calls.
    ResponseParse,
    /// `tool.execute` (sherma `tool_call`). Fail-closed; policy lives in the guard stage.
    ToolExecute,
    /// `tool.result.size` (R8.6).
    ToolResultSize,
    /// `a2a.send` (sherma `agent_call`). Fail-closed.
    A2aSend,
    /// `skill.load`.
    SkillLoad,
    /// `skill.unload`.
    SkillUnload,
    /// `skill.invoke` (R6.4).
    SkillInvoke,
    /// `state.persist` (R14.2). Fail-closed.
    StatePersist,
    /// `plan.persist` (R14.3).
    PlanPersist,
    /// `response.deliver`.
    ResponseDeliver,
    /// `command.invoke` (R10.4).
    CommandInvoke,
    /// `participant.ask` (R18.4).
    ParticipantAsk,
    /// `participant.delegate` (R18.4).
    ParticipantDelegate,
    /// `durable.activity_run` (R22.3).
    DurableActivityRun,
    /// `durable.activity_failure` (R22.3).
    DurableActivityFailure,
    /// `durable.activity_retry` (R22.3).
    DurableActivityRetry,
    /// `heartbeat.tick` (R23).
    HeartbeatTick,
    /// `task.stalled` (R23.3).
    TaskStalled,
    /// `harness.error` (sherma `error`).
    HarnessError,
    /// `infra.http` (R4.7).
    InfraHttp,
    /// `infra.clock` (R4.7).
    InfraClock,
    /// `infra.random` (R4.7).
    InfraRandom,
    /// `infra.secret` (R4.7).
    InfraSecret,
}

impl HookPoint {
    /// Every point, in catalogue order.
    pub const ALL: [HookPoint; 40] = [
        Self::HarnessStart,
        Self::HarnessStop,
        Self::RequestReceive,
        Self::InboxAccept,
        Self::InboxDeliver,
        Self::TaskCreate,
        Self::TaskStateChange,
        Self::TaskRun,
        Self::StepEnter,
        Self::StepExecute,
        Self::StepExit,
        Self::StepError,
        Self::PromptAssemble,
        Self::ContextBuild,
        Self::ContextCompact,
        Self::ModelCreate,
        Self::ModelCall,
        Self::ResponseParse,
        Self::ToolExecute,
        Self::ToolResultSize,
        Self::A2aSend,
        Self::SkillLoad,
        Self::SkillUnload,
        Self::SkillInvoke,
        Self::StatePersist,
        Self::PlanPersist,
        Self::ResponseDeliver,
        Self::CommandInvoke,
        Self::ParticipantAsk,
        Self::ParticipantDelegate,
        Self::DurableActivityRun,
        Self::DurableActivityFailure,
        Self::DurableActivityRetry,
        Self::HeartbeatTick,
        Self::TaskStalled,
        Self::HarnessError,
        Self::InfraHttp,
        Self::InfraClock,
        Self::InfraRandom,
        Self::InfraSecret,
    ];

    /// The dotted name of the point (`tool.execute`), as remote hooks and traces see it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::HarnessStart => "harness.start",
            Self::HarnessStop => "harness.stop",
            Self::RequestReceive => "request.receive",
            Self::InboxAccept => "inbox.accept",
            Self::InboxDeliver => "inbox.deliver",
            Self::TaskCreate => "task.create",
            Self::TaskStateChange => "task.state_change",
            Self::TaskRun => "task.run",
            Self::StepEnter => "step.enter",
            Self::StepExecute => "step.execute",
            Self::StepExit => "step.exit",
            Self::StepError => "step.error",
            Self::PromptAssemble => "prompt.assemble",
            Self::ContextBuild => "context.build",
            Self::ContextCompact => "context.compact",
            Self::ModelCreate => "model.create",
            Self::ModelCall => "model.call",
            Self::ResponseParse => "response.parse",
            Self::ToolExecute => "tool.execute",
            Self::ToolResultSize => "tool.result.size",
            Self::A2aSend => "a2a.send",
            Self::SkillLoad => "skill.load",
            Self::SkillUnload => "skill.unload",
            Self::SkillInvoke => "skill.invoke",
            Self::StatePersist => "state.persist",
            Self::PlanPersist => "plan.persist",
            Self::ResponseDeliver => "response.deliver",
            Self::CommandInvoke => "command.invoke",
            Self::ParticipantAsk => "participant.ask",
            Self::ParticipantDelegate => "participant.delegate",
            Self::DurableActivityRun => "durable.activity_run",
            Self::DurableActivityFailure => "durable.activity_failure",
            Self::DurableActivityRetry => "durable.activity_retry",
            Self::HeartbeatTick => "heartbeat.tick",
            Self::TaskStalled => "task.stalled",
            Self::HarnessError => "harness.error",
            Self::InfraHttp => "infra.http",
            Self::InfraClock => "infra.clock",
            Self::InfraRandom => "infra.random",
            Self::InfraSecret => "infra.secret",
        }
    }

    /// The point with this dotted name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// Points whose `pre`-stage policy is fail-closed and cannot be lowered: tool
    /// execution, outbound A2A, persistence and the durable store of inbox events
    /// (R4.6, design.md § Security design).
    #[must_use]
    pub const fn is_security_relevant(self) -> bool {
        matches!(
            self,
            Self::ToolExecute | Self::A2aSend | Self::StatePersist | Self::InboxAccept
        )
    }

    /// The default failure policy for `pre` and `post` hooks on this point.
    #[must_use]
    pub const fn default_policy(self) -> FailurePolicy {
        if self.is_security_relevant() {
            FailurePolicy::FailClosed
        } else {
            FailurePolicy::LogAndContinue
        }
    }
}

impl fmt::Display for HookPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The four stages of a dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    /// May rewrite the input or veto.
    Pre,
    /// Policy: allow or deny the final input; never mutates; always fail-closed.
    Guard,
    /// Wraps or replaces the implementation.
    In,
    /// May rewrite the output.
    Post,
}

/// What happens when a `pre` or `post` hook fails (R4.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailurePolicy {
    /// The operation does not run.
    FailClosed,
    /// The failure is logged and the chain continues with the unchanged value.
    LogAndContinue,
}

/// One catalogue entry, as a type: its point and its typed input and output.
pub trait Operation: 'static {
    /// The point this operation is dispatched at.
    const POINT: HookPoint;
    /// What the operation consumes; serializable so remote hooks see the same value.
    type Input: Serialize + DeserializeOwned + Clone + MaybeSend + 'static;
    /// What it produces.
    type Output: Serialize + DeserializeOwned + Clone + MaybeSend + 'static;

    /// The identity fields of an input (a tool name, an agent id, a store key): what a
    /// guard approved and what an `in` hook must not move. Empty when the operation has
    /// no target.
    fn identity(_input: &Self::Input) -> String {
        String::new()
    }
}

/// Why a hook said no.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Veto {
    /// A string-catalog key naming the reason, for the person who sees it.
    pub reason: String,
    /// Free text for the trace.
    pub detail: Option<String>,
    /// The hook that vetoed.
    pub by: EntityId,
}

/// A `pre` hook's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreOutcome<I> {
    /// Continue with this (possibly rewritten) input.
    Continue(I),
    /// Do not run the operation.
    Veto(Veto),
}

/// A guard's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardOutcome {
    /// Dispatch may proceed.
    Allow,
    /// Dispatch must not proceed.
    Deny(Veto),
}

/// What a hook reports when it cannot do its job.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[non_exhaustive]
pub enum HookError {
    /// The hook itself failed.
    #[error("{0}")]
    Failed(String),
    /// An `in` hook called `next` with a different target than the guard approved.
    #[error("the in hook moved the target from {expected:?} to {actual:?}")]
    TargetChanged {
        /// The identity the guard stage approved.
        expected: String,
        /// The identity the `in` hook tried to dispatch.
        actual: String,
    },
    /// A remote hook could not be reached or answered badly.
    #[error("remote hook: {0}")]
    Remote(String),
}

/// What every hook sees besides its input.
#[derive(Clone)]
pub struct HookCx {
    /// The task, when the operation belongs to one.
    pub task: Option<String>,
    /// The A2A context id, when known.
    pub context_id: Option<String>,
    /// The W3C `traceparent` of the enclosing span, persisted across signals and workers.
    pub traceparent: Option<String>,
    /// The registry, for hooks that resolve entities.
    pub registry: Arc<Registry>,
    /// The host, for hooks that need the platform.
    pub host: Arc<Host>,
}

impl fmt::Debug for HookCx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HookCx")
            .field("task", &self.task)
            .field("context_id", &self.context_id)
            .finish_non_exhaustive()
    }
}

impl HookCx {
    /// A context outside any task.
    #[must_use]
    pub fn new(registry: Arc<Registry>, host: Arc<Host>) -> Self {
        Self {
            task: None,
            context_id: None,
            traceparent: None,
            registry,
            host,
        }
    }
}

harness_async_trait! {
    /// Runs before the operation; may rewrite the input or veto (R4.2).
    pub trait PreHook<O: Operation>: Send + Sync {
        /// The hook.
        async fn pre(&self, cx: &HookCx, input: O::Input) -> Result<PreOutcome<O::Input>, HookError>;
    }
}

harness_async_trait! {
    /// Policy on the final input: allow or deny, never mutate. Always fail-closed.
    pub trait GuardHook<O: Operation>: Send + Sync {
        /// The guard.
        async fn guard(&self, cx: &HookCx, input: &O::Input) -> Result<GuardOutcome, HookError>;
    }
}

/// The rest of the `in` chain, ending in the default implementation. An `in` hook calls
/// it (possibly with a rewritten input that keeps the same identity) or replaces it.
#[cfg(not(target_arch = "wasm32"))]
pub type Next<'a, O> = Box<
    dyn FnOnce(
            <O as Operation>::Input,
        ) -> HarnessFuture<'a, Result<<O as Operation>::Output, HarnessError>>
        + Send
        + 'a,
>;

/// The rest of the `in` chain, ending in the default implementation (`wasm32`: not `Send`).
#[cfg(target_arch = "wasm32")]
pub type Next<'a, O> = Box<
    dyn FnOnce(
            <O as Operation>::Input,
        ) -> HarnessFuture<'a, Result<<O as Operation>::Output, HarnessError>>
        + 'a,
>;

harness_async_trait! {
    /// Wraps or replaces the implementation (R4.3).
    pub trait InHook<O: Operation>: Send + Sync {
        /// The hook: call `next` to continue, or answer without it.
        async fn run<'a>(&self, cx: &'a HookCx, input: O::Input, next: Next<'a, O>) -> Result<O::Output, HarnessError>;
    }
}

harness_async_trait! {
    /// Runs after the operation; may rewrite the output (R4.4).
    pub trait PostHook<O: Operation>: Send + Sync {
        /// The hook.
        async fn post(&self, cx: &HookCx, input: &O::Input, output: O::Output) -> Result<O::Output, HookError>;
    }
}

/// Where a hook sits among the others on its point and phase (R4.5): lower priority runs
/// first, `after` names hooks that must run before it, registration order breaks ties.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookOrder {
    /// Lower runs first; default 0.
    pub priority: i32,
    /// Hooks (by their registered id) that must have run before this one.
    pub after: Vec<EntityId>,
}

struct Registered<H> {
    id: EntityId,
    order: HookOrder,
    seq: u64,
    hook: H,
}

struct Chain<O: Operation> {
    pre: Vec<Registered<Arc<dyn PreHook<O>>>>,
    guard: Vec<Registered<Arc<dyn GuardHook<O>>>>,
    r#in: Vec<Registered<Arc<dyn InHook<O>>>>,
    post: Vec<Registered<Arc<dyn PostHook<O>>>>,
}

impl<O: Operation> Default for Chain<O> {
    fn default() -> Self {
        Self {
            pre: Vec::new(),
            guard: Vec::new(),
            r#in: Vec::new(),
            post: Vec::new(),
        }
    }
}

/// The hooks of one phase, in dispatch order.
type Picked<H> = Vec<(EntityId, H)>;

/// The four phases of one operation, in dispatch order.
struct Snapshot<O: Operation> {
    pre: Picked<Arc<dyn PreHook<O>>>,
    guard: Picked<Arc<dyn GuardHook<O>>>,
    r#in: Picked<Arc<dyn InHook<O>>>,
    post: Picked<Arc<dyn PostHook<O>>>,
}

impl<O: Operation> Default for Snapshot<O> {
    fn default() -> Self {
        Self {
            pre: Vec::new(),
            guard: Vec::new(),
            r#in: Vec::new(),
            post: Vec::new(),
        }
    }
}

fn pick<H: Clone>(items: &[Registered<H>]) -> Picked<H> {
    sorted(items)
        .into_iter()
        .map(|i| (items[i].id.clone(), items[i].hook.clone()))
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
type AnyBox = Box<dyn Any + Send + Sync>;
#[cfg(target_arch = "wasm32")]
type AnyBox = Box<dyn Any>;

/// Runs every operation through its hooks (R4).
#[derive(Default)]
pub struct HookDispatcher {
    chains: RwLock<BTreeMap<TypeId, AnyBox>>,
    policies: RwLock<BTreeMap<HookPoint, FailurePolicy>>,
    seq: RwLock<u64>,
}

impl fmt::Debug for HookDispatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let operations = self.chains.read().map_or(0, |c| c.len());
        f.debug_struct("HookDispatcher")
            .field("operations", &operations)
            .finish_non_exhaustive()
    }
}

/// A hook, in the phase it registers for.
pub enum Hook<O: Operation> {
    /// A `pre` hook.
    Pre(Arc<dyn PreHook<O>>),
    /// A guard.
    Guard(Arc<dyn GuardHook<O>>),
    /// An `in` hook.
    In(Arc<dyn InHook<O>>),
    /// A `post` hook.
    Post(Arc<dyn PostHook<O>>),
}

impl<O: Operation> fmt::Debug for Hook<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pre(_) => "Hook::Pre",
            Self::Guard(_) => "Hook::Guard",
            Self::In(_) => "Hook::In",
            Self::Post(_) => "Hook::Post",
        })
    }
}

/// A policy change the dispatcher refuses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0} is security-relevant: its policy is fail-closed and cannot be lowered")]
pub struct PolicyLocked(pub HookPoint);

fn sorted<H>(items: &[Registered<H>]) -> Vec<usize> {
    // Kahn's algorithm over the `after` edges, picking the lowest (priority, seq) each time.
    let n = items.len();
    let mut indegree = vec![0usize; n];
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, item) in items.iter().enumerate() {
        for dep in &item.order.after {
            if let Some(j) = items.iter().position(|other| &other.id == dep) {
                edges[j].push(i);
                indegree[i] += 1;
            }
        }
    }
    let mut ready: Vec<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
    let mut out = Vec::with_capacity(n);
    while !ready.is_empty() {
        ready.sort_by_key(|&i| (items[i].order.priority, items[i].seq));
        let next = ready.remove(0);
        out.push(next);
        for &dependent in &edges[next] {
            indegree[dependent] -= 1;
            if indegree[dependent] == 0 {
                ready.push(dependent);
            }
        }
    }
    // A cycle leaves hooks behind; they run last in registration order rather than never.
    for i in 0..n {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

impl HookDispatcher {
    /// An empty dispatcher with the catalogue's default policies.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn next_seq(&self) -> u64 {
        let mut seq = self
            .seq
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *seq += 1;
        *seq
    }

    fn with_chain<O: Operation, R>(&self, f: impl FnOnce(&mut Chain<O>) -> R) -> R {
        let mut chains = self
            .chains
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = chains
            .entry(TypeId::of::<O>())
            .or_insert_with(|| Box::new(Chain::<O>::default()));
        let chain = entry
            .downcast_mut::<Chain<O>>()
            .expect("a chain is stored under its operation's type id");
        f(chain)
    }

    /// Registers `hook` on `O`'s point under `id`, in its phase (R4.5, R4.9). The id is
    /// what other hooks name in `HookOrder::after`.
    pub fn register<O: Operation>(
        &self,
        id: EntityId,
        hook: Hook<O>,
        order: HookOrder,
    ) -> EntityId {
        let seq = self.next_seq();
        self.with_chain::<O, _>(|chain| match hook {
            Hook::Pre(h) => chain.pre.push(Registered {
                id: id.clone(),
                order,
                seq,
                hook: h,
            }),
            Hook::Guard(h) => chain.guard.push(Registered {
                id: id.clone(),
                order,
                seq,
                hook: h,
            }),
            Hook::In(h) => chain.r#in.push(Registered {
                id: id.clone(),
                order,
                seq,
                hook: h,
            }),
            Hook::Post(h) => chain.post.push(Registered {
                id: id.clone(),
                order,
                seq,
                hook: h,
            }),
        });
        id
    }

    /// Sets the `pre`/`post` failure policy of a point (R4.6).
    ///
    /// # Errors
    ///
    /// [`PolicyLocked`] for the security-relevant points, which stay fail-closed.
    pub fn set_policy(&self, point: HookPoint, policy: FailurePolicy) -> Result<(), PolicyLocked> {
        if point.is_security_relevant() && policy != FailurePolicy::FailClosed {
            return Err(PolicyLocked(point));
        }
        self.policies
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(point, policy);
        Ok(())
    }

    /// The policy in force for a point.
    #[must_use]
    pub fn policy(&self, point: HookPoint) -> FailurePolicy {
        self.policies
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&point)
            .copied()
            .unwrap_or_else(|| point.default_policy())
    }

    fn snapshot<O: Operation>(&self) -> Snapshot<O> {
        let chains = self
            .chains
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(chain) = chains
            .get(&TypeId::of::<O>())
            .and_then(|c| c.downcast_ref::<Chain<O>>())
        else {
            return Snapshot::default();
        };
        Snapshot {
            pre: pick(&chain.pre),
            guard: pick(&chain.guard),
            r#in: pick(&chain.r#in),
            post: pick(&chain.post),
        }
    }

    /// Runs `O` on `input` through every registered hook, with `default` as the
    /// implementation the innermost `in` hook wraps.
    ///
    /// # Errors
    ///
    /// [`HarnessError::Vetoed`] when a `pre` hook or a guard says no;
    /// [`HarnessError::Hook`] when a hook fails under a fail-closed policy or an `in`
    /// hook moves the target; whatever `default` or an `in` hook returns otherwise.
    pub async fn run<'a, O, F, Fut>(
        &self,
        cx: &'a HookCx,
        input: O::Input,
        default: F,
    ) -> Result<O::Output, HarnessError>
    where
        O: Operation,
        F: FnOnce(O::Input) -> Fut + MaybeSend + 'a,
        Fut: core::future::Future<Output = Result<O::Output, HarnessError>> + MaybeSend + 'a,
    {
        let point = O::POINT;
        let policy = self.policy(point);
        let Snapshot {
            pre: pres,
            guard: guards,
            r#in: ins,
            post: posts,
        } = self.snapshot::<O>();

        let mut input = input;
        for (id, hook) in pres {
            match hook.pre(cx, input.clone()).await {
                Ok(PreOutcome::Continue(next)) => input = next,
                Ok(PreOutcome::Veto(veto)) => return Err(HarnessError::Vetoed(veto)),
                Err(source) => match policy {
                    FailurePolicy::FailClosed => {
                        return Err(HarnessError::Hook {
                            point,
                            phase: Phase::Pre,
                            source,
                            policy,
                        });
                    }
                    FailurePolicy::LogAndContinue => {
                        tracing::warn!(point = point.name(), hook = %id, error = %source, "pre hook failed; continuing");
                    }
                },
            }
        }

        for (_, guard) in guards {
            match guard.guard(cx, &input).await {
                Ok(GuardOutcome::Allow) => {}
                Ok(GuardOutcome::Deny(veto)) => return Err(HarnessError::Vetoed(veto)),
                Err(source) => {
                    return Err(HarnessError::Hook {
                        point,
                        phase: Phase::Guard,
                        source,
                        policy: FailurePolicy::FailClosed,
                    });
                }
            }
        }

        let approved = O::identity(&input);
        let next = checked_default::<O, _, _>(approved.clone(), default);
        let output = run_in_chain::<O>(cx, ins, input.clone(), next, approved).await?;

        let mut output = output;
        for (id, hook) in posts {
            match hook.post(cx, &input, output.clone()).await {
                Ok(next) => output = next,
                Err(source) => match policy {
                    FailurePolicy::FailClosed => {
                        return Err(HarnessError::Hook {
                            point,
                            phase: Phase::Post,
                            source,
                            policy,
                        });
                    }
                    FailurePolicy::LogAndContinue => {
                        tracing::warn!(point = point.name(), hook = %id, error = %source, "post hook failed; continuing");
                    }
                },
            }
        }
        Ok(output)
    }
}

/// The default implementation behind the `in` chain, refusing an input whose identity is
/// not the one the guard stage approved.
fn checked_default<'a, O, F, Fut>(approved: String, default: F) -> Next<'a, O>
where
    O: Operation,
    F: FnOnce(O::Input) -> Fut + MaybeSend + 'a,
    Fut: core::future::Future<Output = Result<O::Output, HarnessError>> + MaybeSend + 'a,
{
    Box::new(
        move |input: O::Input| -> HarnessFuture<'a, Result<O::Output, HarnessError>> {
            let actual = O::identity(&input);
            if actual != approved {
                return Box::pin(async move {
                    Err(HarnessError::Hook {
                        point: O::POINT,
                        phase: Phase::In,
                        source: HookError::TargetChanged {
                            expected: approved,
                            actual,
                        },
                        policy: FailurePolicy::FailClosed,
                    })
                });
            }
            Box::pin(default(input))
        },
    )
}

/// Runs the `in` hooks outermost-first, each receiving the rest of the chain as `next`.
fn run_in_chain<'a, O: Operation>(
    cx: &'a HookCx,
    mut hooks: Vec<(EntityId, Arc<dyn InHook<O>>)>,
    input: O::Input,
    next: Next<'a, O>,
    approved: String,
) -> HarnessFuture<'a, Result<O::Output, HarnessError>> {
    if hooks.is_empty() {
        return next(input);
    }
    let (_, hook) = hooks.remove(0);
    let rest: Next<'a, O> = Box::new(move |input: O::Input| {
        let actual = O::identity(&input);
        if actual != approved {
            return Box::pin(async move {
                Err(HarnessError::Hook {
                    point: O::POINT,
                    phase: Phase::In,
                    source: HookError::TargetChanged {
                        expected: approved,
                        actual,
                    },
                    policy: FailurePolicy::FailClosed,
                })
            });
        }
        run_in_chain::<O>(cx, hooks, input, next, approved)
    });
    Box::pin(async move { hook.run(cx, input, rest).await })
}

#[cfg(test)]
mod tests {
    use super::{
        FailurePolicy, GuardHook, GuardOutcome, Hook, HookCx, HookDispatcher, HookError, HookOrder,
        HookPoint, InHook, Next, Operation, Phase, PostHook, PreHook, PreOutcome, Veto,
    };
    use crate::error::HarnessError;
    use crate::harness_async_trait;
    use crate::host::Host;
    use crate::registry::{EntityId, Registry};
    use std::sync::{Arc, Mutex};

    /// `tool.execute` stand-in: the input is "<tool> <args>", the identity is the tool.
    struct Exec;
    impl Operation for Exec {
        const POINT: HookPoint = HookPoint::ToolExecute;
        type Input = String;
        type Output = String;
        fn identity(input: &String) -> String {
            input.split_whitespace().next().unwrap_or("").to_owned()
        }
    }

    /// A point with a log-and-continue default.
    struct Deliver;
    impl Operation for Deliver {
        const POINT: HookPoint = HookPoint::ResponseDeliver;
        type Input = String;
        type Output = String;
    }

    struct Tag(&'static str, Arc<Mutex<Vec<&'static str>>>);
    harness_async_trait! {
        impl<O: Operation<Input = String>> PreHook<O> for Tag {
            async fn pre(&self, _cx: &HookCx, input: String) -> Result<PreOutcome<String>, HookError> {
                self.1.lock().unwrap().push(self.0);
                Ok(PreOutcome::Continue(format!("{input} +{}", self.0)))
            }
        }
    }

    struct Rewrite(&'static str);
    harness_async_trait! {
        impl PreHook<Exec> for Rewrite {
            async fn pre(&self, _cx: &HookCx, _input: String) -> Result<PreOutcome<String>, HookError> {
                Ok(PreOutcome::Continue(self.0.to_owned()))
            }
        }
    }

    struct Failing;
    harness_async_trait! {
        impl<O: Operation<Input = String>> PreHook<O> for Failing {
            async fn pre(&self, _cx: &HookCx, _input: String) -> Result<PreOutcome<String>, HookError> {
                Err(HookError::Failed("boom".into()))
            }
        }
    }

    struct DenyShell;
    harness_async_trait! {
        impl GuardHook<Exec> for DenyShell {
            async fn guard(&self, _cx: &HookCx, input: &String) -> Result<GuardOutcome, HookError> {
                Ok(if input.starts_with("shell") {
                    GuardOutcome::Deny(Veto { reason: "tool.denied".into(), detail: None, by: EntityId::new("policy") })
                } else {
                    GuardOutcome::Allow
                })
            }
        }
    }

    struct Wrap(&'static str);
    harness_async_trait! {
        impl InHook<Exec> for Wrap {
            async fn run<'a>(&self, _cx: &'a HookCx, input: String, next: Next<'a, Exec>) -> Result<String, HarnessError> {
                let out = next(input).await?;
                Ok(format!("{}({out})", self.0))
            }
        }
    }

    struct Replace;
    harness_async_trait! {
        impl InHook<Exec> for Replace {
            async fn run<'a>(&self, _cx: &'a HookCx, _input: String, _next: Next<'a, Exec>) -> Result<String, HarnessError> {
                Ok("replaced".into())
            }
        }
    }

    struct MoveTarget;
    harness_async_trait! {
        impl InHook<Exec> for MoveTarget {
            async fn run<'a>(&self, _cx: &'a HookCx, _input: String, next: Next<'a, Exec>) -> Result<String, HarnessError> {
                next("shell rm -rf".into()).await
            }
        }
    }

    struct Upper;
    harness_async_trait! {
        impl<O: Operation<Input = String, Output = String>> PostHook<O> for Upper {
            async fn post(&self, _cx: &HookCx, _input: &String, output: String) -> Result<String, HookError> {
                Ok(output.to_uppercase())
            }
        }
    }

    fn cx() -> HookCx {
        HookCx::new(Arc::new(Registry::new()), Arc::new(Host::testing()))
    }

    async fn echo(input: String) -> Result<String, HarnessError> {
        Ok(format!("ran[{input}]"))
    }

    #[tokio::test]
    async fn pre_hooks_run_by_priority_then_after_then_registration() {
        let d = HookDispatcher::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let c = EntityId::new("c");
        d.register::<Exec>(
            c.clone(),
            Hook::Pre(Arc::new(Tag("c", seen.clone()))),
            HookOrder {
                priority: 5,
                after: vec![],
            },
        );
        d.register::<Exec>(
            EntityId::new("a"),
            Hook::Pre(Arc::new(Tag("a", seen.clone()))),
            HookOrder::default(),
        );
        d.register::<Exec>(
            EntityId::new("b"),
            Hook::Pre(Arc::new(Tag("b", seen.clone()))),
            HookOrder {
                priority: 0,
                after: vec![c],
            },
        );
        let out = d
            .run::<Exec, _, _>(&cx(), "echo hi".into(), echo)
            .await
            .expect("runs");
        assert_eq!(seen.lock().unwrap().as_slice(), ["a", "c", "b"]);
        assert_eq!(out, "ran[echo hi +a +c +b]");
    }

    #[tokio::test]
    async fn a_pre_veto_stops_the_operation_before_it_runs() {
        struct Nope;
        harness_async_trait! {
            impl PreHook<Exec> for Nope {
                async fn pre(&self, _cx: &HookCx, _input: String) -> Result<PreOutcome<String>, HookError> {
                    Ok(PreOutcome::Veto(Veto { reason: "no".into(), detail: Some("test".into()), by: EntityId::new("nope") }))
                }
            }
        }
        let d = HookDispatcher::new();
        d.register::<Exec>(
            EntityId::new("nope"),
            Hook::Pre(Arc::new(Nope)),
            HookOrder::default(),
        );
        let ran = Arc::new(Mutex::new(false));
        let flag = ran.clone();
        let err = d
            .run::<Exec, _, _>(&cx(), "echo".into(), move |_| async move {
                *flag.lock().unwrap() = true;
                Ok(String::new())
            })
            .await
            .expect_err("vetoed");
        assert!(matches!(err, HarnessError::Vetoed(v) if v.reason == "no"));
        assert!(!*ran.lock().unwrap());
    }

    #[tokio::test]
    async fn a_pre_hook_that_rewrites_a_tool_call_after_approval_is_rechecked_by_the_guard() {
        // Scenario: A pre hook that rewrites a tool call after approval is re-checked by the guard
        //   Given a guard that denies the `shell` tool
        //   And a later pre hook that rewrites any call into `shell rm -rf`
        //   When the model calls `echo hi`
        //   Then the guard denies the rewritten call and nothing runs
        let d = HookDispatcher::new();
        d.register::<Exec>(
            EntityId::new("policy"),
            Hook::Guard(Arc::new(DenyShell)),
            HookOrder::default(),
        );
        d.register::<Exec>(
            EntityId::new("late"),
            Hook::Pre(Arc::new(Rewrite("shell rm -rf"))),
            HookOrder {
                priority: 100,
                after: vec![],
            },
        );
        let err = d
            .run::<Exec, _, _>(&cx(), "echo hi".into(), echo)
            .await
            .expect_err("denied");
        assert!(matches!(err, HarnessError::Vetoed(v) if v.reason == "tool.denied"));
    }

    #[tokio::test]
    async fn in_hooks_wrap_outermost_first_and_may_replace() {
        let d = HookDispatcher::new();
        d.register::<Exec>(
            EntityId::new("outer"),
            Hook::In(Arc::new(Wrap("outer"))),
            HookOrder::default(),
        );
        d.register::<Exec>(
            EntityId::new("inner"),
            Hook::In(Arc::new(Wrap("inner"))),
            HookOrder::default(),
        );
        let out = d
            .run::<Exec, _, _>(&cx(), "echo".into(), echo)
            .await
            .expect("runs");
        assert_eq!(out, "outer(inner(ran[echo]))");
        let d = HookDispatcher::new();
        d.register::<Exec>(
            EntityId::new("replace"),
            Hook::In(Arc::new(Replace)),
            HookOrder::default(),
        );
        assert_eq!(
            d.run::<Exec, _, _>(&cx(), "echo".into(), echo)
                .await
                .expect("replaced"),
            "replaced"
        );
    }

    #[tokio::test]
    async fn an_in_hook_that_moves_the_target_fails_the_operation() {
        let d = HookDispatcher::new();
        d.register::<Exec>(
            EntityId::new("policy"),
            Hook::Guard(Arc::new(DenyShell)),
            HookOrder::default(),
        );
        d.register::<Exec>(
            EntityId::new("mover"),
            Hook::In(Arc::new(MoveTarget)),
            HookOrder::default(),
        );
        let err = d
            .run::<Exec, _, _>(&cx(), "echo hi".into(), echo)
            .await
            .expect_err("moved");
        assert!(matches!(
            err,
            HarnessError::Hook { phase: Phase::In, source: HookError::TargetChanged { ref expected, ref actual }, .. }
                if expected == "echo" && actual == "shell"
        ));
    }

    #[tokio::test]
    async fn post_hooks_rewrite_the_output() {
        let d = HookDispatcher::new();
        d.register::<Exec>(
            EntityId::new("upper"),
            Hook::Post(Arc::new(Upper)),
            HookOrder::default(),
        );
        assert_eq!(
            d.run::<Exec, _, _>(&cx(), "echo".into(), echo)
                .await
                .expect("runs"),
            "RAN[ECHO]"
        );
    }

    #[tokio::test]
    async fn pre_failures_follow_the_point_policy_and_security_points_stay_closed() {
        let d = HookDispatcher::new();
        d.register::<Exec>(
            EntityId::new("bad"),
            Hook::Pre(Arc::new(Failing)),
            HookOrder::default(),
        );
        let err = d
            .run::<Exec, _, _>(&cx(), "echo".into(), echo)
            .await
            .expect_err("fail closed");
        assert!(matches!(
            err,
            HarnessError::Hook {
                point: HookPoint::ToolExecute,
                phase: Phase::Pre,
                policy: FailurePolicy::FailClosed,
                ..
            }
        ));
        assert_eq!(
            d.set_policy(HookPoint::ToolExecute, FailurePolicy::LogAndContinue)
                .expect_err("locked")
                .0,
            HookPoint::ToolExecute
        );

        d.register::<Deliver>(
            EntityId::new("bad"),
            Hook::Pre(Arc::new(Failing)),
            HookOrder::default(),
        );
        assert_eq!(
            d.policy(HookPoint::ResponseDeliver),
            FailurePolicy::LogAndContinue
        );
        assert_eq!(
            d.run::<Deliver, _, _>(&cx(), "msg".into(), echo)
                .await
                .expect("continues"),
            "ran[msg]"
        );
        d.set_policy(HookPoint::ResponseDeliver, FailurePolicy::FailClosed)
            .expect("allowed");
        assert!(
            d.run::<Deliver, _, _>(&cx(), "msg".into(), echo)
                .await
                .is_err()
        );
    }

    #[test]
    fn the_catalogue_names_are_unique_dotted_and_round_trip() {
        let names: std::collections::BTreeSet<_> =
            HookPoint::ALL.iter().map(|p| p.name()).collect();
        assert_eq!(names.len(), HookPoint::ALL.len());
        for point in HookPoint::ALL {
            assert!(point.name().contains('.'), "{point}");
            assert_eq!(HookPoint::from_name(point.name()), Some(point));
        }
        assert!(
            HookPoint::ToolExecute.is_security_relevant()
                && HookPoint::A2aSend.is_security_relevant()
        );
        assert_eq!(
            HookPoint::ModelCall.default_policy(),
            FailurePolicy::LogAndContinue
        );
    }
}

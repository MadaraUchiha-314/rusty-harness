//! The A2A 1.0 server on the official Rust SDK: the `AgentExecutor` adapter, extensions,
//! authentication, bindings and the marked `protocol_gap` module (R19); compiled with the
//! `server` feature.
//!
//! [`HarnessExecutor`] is the SDK's `AgentExecutor`: `execute` hands the message to the
//! local agent and returns the task's event log as the stream (the per-subscriber
//! projection), `cancel` runs the stop path and yields the final task.
//! [`SdkTaskStore`] is the SDK's `TaskStore` over the harness's [`PersistenceStore`]: the
//! journal the loop writes is the one the SDK reads, so a restart serves the full task
//! (R12.2, R14.4). [`AgentCardBuilder`] builds the card from the configuration and the
//! registry (R19.2); [`router`] mounts the JSON-RPC and HTTP+JSON bindings and the card
//! route, and [`serve`] binds them (loopback only without a security scheme).

use std::collections::HashMap;
use std::sync::Arc;

use a2a_server::{
    AgentExecutor, ExecutorContext, RequestHandler, ServiceParams, StaticAgentCard, TaskStore,
};
use futures::StreamExt as _;
use futures::stream::BoxStream;

use crate::Harness;
use crate::config::{Config, SecuritySchemeConfig};
use crate::harness::core_loop::agent::{Agent as _, AgentRequest, CallContext, LocalAgent};
use crate::harness::core_loop::persistence::{PersistenceStore, TaskQuery};
use crate::harness::core_loop::task::{Caller, Role, ext};
use crate::harness::skills::Skill;
use crate::registry::{Entity, Registry};

pub mod protocol_gap;

/// The A2UI extension the card advertises beside the harness's own (R25.4).
pub const A2UI_EXT: &str = crate::harness::channels::A2UI_EXT;

/// The SDK's `AgentExecutor` over the local agent (design.md § A2A API server).
pub struct HarnessExecutor {
    agent: Arc<LocalAgent>,
    inbox: Arc<crate::app_service::inbox::Inbox>,
}

impl core::fmt::Debug for HarnessExecutor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HarnessExecutor").finish_non_exhaustive()
    }
}

impl HarnessExecutor {
    /// The executor over `agent`, ingesting through `inbox`.
    #[must_use]
    pub fn new(agent: Arc<LocalAgent>, inbox: Arc<crate::app_service::inbox::Inbox>) -> Self {
        Self { agent, inbox }
    }

    /// The caller of a request: the authenticated principal when the service layer set
    /// one (task 21), else the SDK's user, else the loopback operator.
    fn caller(ctx: &ExecutorContext) -> Caller {
        if let Some(id) = protocol_gap::caller_id(&ctx.service_params) {
            return Caller::human(id, [Role::Admin]);
        }
        match &ctx.user {
            Some(user) if user.authenticated => Caller::human(user.name.clone(), [Role::Admin]),
            _ => Caller::human("operator", [Role::Admin]),
        }
    }
}

/// The task's log as a stream for `viewer`: the Task snapshot first (redacted for a
/// non-member of a channel), then the stored events from `from` without the raw
/// `Message` entries, following live commits until the turn ends (R19.7, R19.8).
pub async fn log_stream(
    store: Arc<dyn PersistenceStore>,
    task: String,
    from: u64,
    viewer: Caller,
) -> BoxStream<'static, Result<a2a::StreamResponse, a2a::A2AError>> {
    let first: Vec<Result<a2a::StreamResponse, a2a::A2AError>> =
        match store.a2a_task(&task, &viewer).await {
            Ok(Some(mut t)) => {
                let _ = crate::app_service::channels_api::redact_for_viewer(
                    store.as_ref(),
                    &mut t,
                    &viewer,
                )
                .await;
                vec![Ok(a2a::StreamResponse::Task(t))]
            }
            Ok(None) => vec![Err(a2a::A2AError::task_not_found(&task))],
            Err(e) => vec![Err(a2a::A2AError::internal(e.to_string()))],
        };
    let is_message = |e: &crate::harness::core_loop::persistence::StoredEvent| {
        matches!(e.response, a2a::StreamResponse::Message(_))
    };
    let live = store.get_task(&task).await.ok().flatten().is_some_and(|r| {
        r.state == a2a::TaskState::Working || r.state == a2a::TaskState::Submitted
    });
    if !live {
        let events = store
            .events(&task, from, usize::MAX, &viewer)
            .await
            .unwrap_or_default();
        return futures::stream::iter(
            first
                .into_iter()
                .chain(
                    events
                        .into_iter()
                        .filter(|e| !is_message(e))
                        .map(|e| Ok(e.response)),
                )
                .collect::<Vec<_>>(),
        )
        .boxed();
    }
    let tail = store
        .tail_events(&task, from, &viewer)
        .filter(move |r| futures::future::ready(!matches!(r, Ok(e) if is_message(e))))
        .map(|r| {
            r.map(|e| e.response)
                .map_err(|e| a2a::A2AError::internal(e.to_string()))
        });
    futures::stream::iter(first)
        .chain(futures::stream::unfold(
            (tail, false),
            |(mut tail, done)| async move {
                if done {
                    return None;
                }
                let item = tail.next().await?;
                let stop = matches!(&item, Ok(e) if turn_ends(e));
                Some((item, (tail, stop)))
            },
        ))
        .boxed()
}

/// Ends a tail after the event that leaves the task waiting or terminal: one turn per
/// request on the wire (R21.3).
fn turn_ends(event: &a2a::StreamResponse) -> bool {
    match event {
        a2a::StreamResponse::StatusUpdate(u) => {
            u.status.state.is_terminal() || u.status.state == a2a::TaskState::InputRequired
        }
        a2a::StreamResponse::Task(t) => t.status.state.is_terminal(),
        _ => false,
    }
}

#[async_trait::async_trait]
impl AgentExecutor for HarnessExecutor {
    #[allow(clippy::too_many_lines)] // one stream, read top to bottom
    fn execute(
        &self,
        ctx: ExecutorContext,
    ) -> BoxStream<'static, Result<a2a::StreamResponse, a2a::A2AError>> {
        let agent = Arc::clone(&self.agent);
        let inbox = Arc::clone(&self.inbox);
        let caller = Self::caller(&ctx);
        let Some(message) = ctx.message else {
            return Box::pin(futures::stream::iter(vec![Err(
                a2a::A2AError::invalid_params("execute without a message"),
            )]));
        };
        let traceparent = crate::app_service::o11y::trace::incoming(
            ctx.service_params
                .get(crate::app_service::o11y::trace::HEADER)
                .and_then(|v| v.first())
                .map(String::as_str),
        );
        let request = AgentRequest {
            message,
            call: CallContext {
                task_id: ctx.task_id.clone(),
                context_id: ctx.context_id.clone(),
                stored_task: ctx.stored_task,
                caller: caller.clone(),
                service_params: ctx
                    .service_params
                    .iter()
                    .map(|(k, v)| (k.clone(), v.join(",")))
                    .collect(),
            },
            metadata: ctx.metadata,
            extensions: protocol_gap::active_extensions(&ctx.service_params),
            inbox_event_id: None,
            traceparent,
        };
        let task = ctx.task_id;
        Box::pin(
            futures::stream::once(async move {
                let store = Arc::clone(agent.store());
                let from = store
                    .events(&task, 1, usize::MAX, &caller)
                    .await
                    .map_or(1, |e| e.len() as u64 + 1);
                // One ingestion path (R20.2): the message is an inbox event; the inbox
                // applies it and signals durable execution, which runs the loop.
                let event = crate::app_service::inbox::InboxEvent {
                    id: if request.message.message_id.is_empty() {
                        a2a::new_message_id()
                    } else {
                        request.message.message_id.clone()
                    },
                    target: crate::app_service::inbox::EventTarget::Task(task.clone()),
                    kind: a2a::StreamResponse::Message(request.message.clone()),
                    status: crate::app_service::inbox::InboxStatus::Received,
                    sender: caller.clone(),
                    received_at: agent
                        .host()
                        .clock()
                        .map_or_else(|_| chrono::Utc::now(), |c| c.now()),
                    delivery: crate::app_service::inbox::DeliveryKind::Interrupt,
                    extensions: request.extensions.clone(),
                    traceparent: request.traceparent.clone(),
                };
                if let Err(e) = inbox.accept(event).await {
                    return futures::stream::iter(vec![Err(e.to_a2a())]).boxed();
                }
                let run = true;
                let viewer = caller.clone().in_channels(
                    crate::harness::channels::Channel::memberships(
                        store.as_ref(),
                        &task,
                        &caller.id,
                    )
                    .await
                    .unwrap_or_default(),
                );
                // Raw `Message` events stay in the log (they are the history); on the wire
                // the reply rides on the status update, because the SDK's non-streaming
                // `SendMessage` takes the first `Message` as the whole answer.
                let is_message = |e: &crate::harness::core_loop::persistence::StoredEvent| {
                    matches!(e.response, a2a::StreamResponse::Message(_))
                };
                // The stream starts with the Task (R19.7).
                let first: Vec<Result<a2a::StreamResponse, a2a::A2AError>> =
                    match store.a2a_task(&task, &viewer).await {
                        Ok(Some(t)) => vec![Ok(a2a::StreamResponse::Task(t))],
                        Ok(None) => Vec::new(),
                        Err(e) => vec![Err(a2a::A2AError::internal(e.to_string()))],
                    };
                if !run {
                    let events = store
                        .events(&task, from, usize::MAX, &viewer)
                        .await
                        .unwrap_or_default();
                    return futures::stream::iter(
                        first
                            .into_iter()
                            .chain(
                                events
                                    .into_iter()
                                    .filter(|e| !is_message(e))
                                    .map(|e| Ok(e.response)),
                            )
                            .collect::<Vec<_>>(),
                    )
                    .boxed();
                }
                let tail = store
                    .tail_events(&task, from, &viewer)
                    .filter(move |r| futures::future::ready(!matches!(r, Ok(e) if is_message(e))))
                    .map(|r| {
                        r.map(|e| e.response)
                            .map_err(|e| a2a::A2AError::internal(e.to_string()))
                    });
                futures::stream::iter(first)
                    .chain(futures::stream::unfold(
                        (tail, false),
                        |(mut tail, done)| async move {
                            if done {
                                return None;
                            }
                            let item = tail.next().await?;
                            let stop = matches!(&item, Ok(e) if turn_ends(e));
                            Some((item, (tail, stop)))
                        },
                    ))
                    .boxed()
            })
            .flatten(),
        )
    }

    fn cancel(
        &self,
        ctx: ExecutorContext,
    ) -> BoxStream<'static, Result<a2a::StreamResponse, a2a::A2AError>> {
        let agent = Arc::clone(&self.agent);
        let caller = Self::caller(&ctx);
        let task = ctx.task_id;
        Box::pin(futures::stream::once(async move {
            agent
                .cancel(
                    &task,
                    crate::app_service::durable::CancelReason::Participant { by: caller.id },
                )
                .await
                .map(a2a::StreamResponse::Task)
                .map_err(|e| e.to_a2a())
        }))
    }
}

/// The SDK's `TaskStore` as a fold over the harness's store (design.md § Task and
/// lifecycle): `get` is the record's A2A projection with its message history,
/// `create` writes the shell record the loop adopts on the first message, `update` is
/// the SDK writing back the harness's own events (accepted, never regressed), and
/// `begin_cancel` is the stop path.
pub struct SdkTaskStore {
    agent: Arc<LocalAgent>,
}

impl core::fmt::Debug for SdkTaskStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SdkTaskStore").finish_non_exhaustive()
    }
}

impl SdkTaskStore {
    /// The store over `agent`'s persistence.
    #[must_use]
    pub fn new(agent: Arc<LocalAgent>) -> Self {
        Self { agent }
    }

    fn store(&self) -> &Arc<dyn PersistenceStore> {
        self.agent.store()
    }

    #[allow(clippy::needless_pass_by_value)] // used as `map_err(Self::internal)`
    fn internal(e: impl ToString) -> a2a::A2AError {
        a2a::A2AError::internal(e.to_string())
    }
}

#[async_trait::async_trait]
impl TaskStore for SdkTaskStore {
    async fn create(
        &self,
        task: a2a::Task,
    ) -> Result<a2a_server::task_store::TaskVersion, a2a::A2AError> {
        if self
            .store()
            .get_task(&task.id)
            .await
            .map_err(Self::internal)?
            .is_some()
        {
            return Err(a2a::A2AError::internal("task already exists"));
        }
        let text = task
            .history
            .as_ref()
            .and_then(|h| h.first())
            .and_then(|m| m.text())
            .unwrap_or_default()
            .to_owned();
        let draft = crate::harness::core_loop::task::TaskDraft {
            id: task.id.clone(),
            context_id: task.context_id.clone(),
            name: text.chars().take(60).collect(),
            r#type: None,
            goal: text,
            description: String::new(),
            acceptance_criteria: Vec::new(),
            mode: crate::harness::core_loop::task::TaskMode::OneShot,
            participants: Vec::new(),
            parent_tasks: Vec::new(),
            reference_task_ids: Vec::new(),
            model: crate::harness::commands::ModelSelection {
                model: "default".into(),
                effort: None,
            },
        };
        let record = crate::harness::core_loop::task::TaskRecord::new(
            draft,
            task.status.timestamp.unwrap_or_else(chrono::Utc::now),
        );
        let seq = self
            .store()
            .commit(crate::harness::core_loop::persistence::TaskCommit::of(
                record,
            ))
            .await
            .map_err(Self::internal)?;
        Ok(seq)
    }

    async fn update(
        &self,
        task: a2a::Task,
    ) -> Result<a2a_server::task_store::TaskVersion, a2a::A2AError> {
        // The events the SDK folds are the harness's own: the record already holds them.
        let record = self
            .store()
            .get_task(&task.id)
            .await
            .map_err(Self::internal)?
            .ok_or_else(|| a2a::A2AError::task_not_found(&task.id))?;
        if record.is_terminal() && record.state != task.status.state {
            return Err(a2a::A2AError::invalid_request(format!(
                "task {} is terminal",
                task.id
            )));
        }
        Ok(record.version)
    }

    async fn get(&self, task_id: &str) -> Result<Option<a2a::Task>, a2a::A2AError> {
        self.store()
            .a2a_task(&task_id.to_owned(), &Caller::system("service"))
            .await
            .map_err(Self::internal)
    }

    async fn list(
        &self,
        req: &a2a::ListTasksRequest,
    ) -> Result<a2a::ListTasksResponse, a2a::A2AError> {
        let mut query = TaskQuery::everyone();
        query.context_id.clone_from(&req.context_id);
        query.states = req
            .status
            .clone()
            .filter(|s| *s != a2a::TaskState::Unspecified)
            .map(|s| vec![s]);
        query.limit = usize::try_from(req.page_size.unwrap_or(50).max(1)).unwrap_or(50);
        query.after.clone_from(&req.page_token);
        let page = self
            .store()
            .list_tasks(query)
            .await
            .map_err(Self::internal)?;
        let mut tasks = Vec::with_capacity(page.items.len());
        for record in &page.items {
            let mut task = record.to_a2a(Vec::new());
            if req.include_artifacts != Some(true) {
                task.artifacts = None;
            }
            tasks.push(task);
        }
        Ok(a2a::ListTasksResponse {
            total_size: i32::try_from(tasks.len()).unwrap_or(i32::MAX),
            page_size: i32::try_from(tasks.len()).unwrap_or(i32::MAX),
            tasks,
            next_page_token: page.next.unwrap_or_default(),
        })
    }

    async fn begin_cancel(&self, task_id: &str) -> Result<a2a::Task, a2a::A2AError> {
        let record = self
            .store()
            .get_task(&task_id.to_owned())
            .await
            .map_err(Self::internal)?
            .ok_or_else(|| a2a::A2AError::task_not_found(task_id))?;
        if record.is_terminal() {
            return Err(a2a::A2AError::task_not_cancelable(task_id));
        }
        self.agent
            .cancel(
                &task_id.to_owned(),
                crate::app_service::durable::CancelReason::Participant {
                    by: "service".into(),
                },
            )
            .await
            .map_err(|e| e.to_a2a())
    }
}

/// The SDK's push-config store behind the operator's allowlist (abuse 9): a webhook host
/// outside `[server.push] allow_hosts` is refused at save time; an empty list refuses
/// every push config; the SDK's own SSRF guard screens the URL and every redirect hop.
pub struct AllowlistedPushStore {
    inner: a2a_server::InMemoryPushConfigStore,
    allow_hosts: Vec<String>,
}

impl core::fmt::Debug for AllowlistedPushStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AllowlistedPushStore")
            .field("allow_hosts", &self.allow_hosts)
            .finish_non_exhaustive()
    }
}

impl AllowlistedPushStore {
    /// A store allowing `allow_hosts` (exact, case-insensitive host names).
    #[must_use]
    pub fn new(allow_hosts: Vec<String>) -> Self {
        Self {
            inner: a2a_server::InMemoryPushConfigStore::new(),
            allow_hosts: allow_hosts
                .into_iter()
                .map(|h| h.to_ascii_lowercase())
                .collect(),
        }
    }

    /// Whether `url` may receive pushes: `https` (or `http`), a host on the allowlist,
    /// never an IP literal.
    ///
    /// # Errors
    ///
    /// [`a2a::A2AError`] (`InvalidParams`) naming the rule.
    pub fn check(&self, url: &str) -> Result<(), a2a::A2AError> {
        let parsed: url::Url = url
            .parse()
            .map_err(|e| a2a::A2AError::invalid_params(format!("push url: {e}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(a2a::A2AError::invalid_params(
                "push url: http or https only",
            ));
        }
        let Some(host) = parsed.host() else {
            return Err(a2a::A2AError::invalid_params("push url: no host"));
        };
        let name = match host {
            url::Host::Domain(d) => d.to_ascii_lowercase(),
            url::Host::Ipv4(_) | url::Host::Ipv6(_) => {
                return Err(a2a::A2AError::invalid_params(
                    "push url: an IP literal is never an allowed webhook host",
                ));
            }
        };
        if !self.allow_hosts.contains(&name) {
            return Err(a2a::A2AError::invalid_params(format!(
                "push url: host {name} is not in [server.push] allow_hosts"
            )));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl a2a_server::PushConfigStore for AllowlistedPushStore {
    async fn save(
        &self,
        config: a2a::TaskPushNotificationConfig,
    ) -> Result<a2a::TaskPushNotificationConfig, a2a::A2AError> {
        self.check(&config.url)?;
        self.inner.save(config).await
    }
    async fn get(
        &self,
        task_id: &str,
        config_id: &str,
    ) -> Result<a2a::TaskPushNotificationConfig, a2a::A2AError> {
        self.inner.get(task_id, config_id).await
    }
    async fn list(
        &self,
        task_id: &str,
    ) -> Result<Vec<a2a::TaskPushNotificationConfig>, a2a::A2AError> {
        self.inner.list(task_id).await
    }
    async fn delete(&self, task_id: &str, config_id: &str) -> Result<(), a2a::A2AError> {
        self.inner.delete(task_id, config_id).await
    }
    async fn delete_all(&self, task_id: &str) -> Result<(), a2a::A2AError> {
        self.inner.delete_all(task_id).await
    }
}

/// Builds the agent card from the configuration and the registry (R19.2).
#[derive(Debug, Default, Clone, Copy)]
pub struct AgentCardBuilder;

impl AgentCardBuilder {
    /// The card: skills from the registered skills, every extension with its URI, the
    /// security schemes of `[server.security]`, one interface per enabled binding.
    #[must_use]
    pub fn build(config: &Config, registry: &Registry, base: &a2a::AgentCard) -> a2a::AgentCard {
        Self::build_at(config, registry, base, &config.server.bind)
    }

    /// The card for a server bound at `bound` (the actual address once bound), unless
    /// `[server] public_url` names where clients reach it.
    ///
    /// # Panics
    ///
    /// When `bound` is not a socket address; the server binds it before building the card.
    #[must_use]
    pub fn build_at(
        config: &Config,
        registry: &Registry,
        base: &a2a::AgentCard,
        bound: &str,
    ) -> a2a::AgentCard {
        let mut card = base.clone();
        let public = config.server.public_url.clone().unwrap_or_else(|| {
            format!("http://{bound}/")
                .parse()
                .expect("a bind address is a URL host")
        });
        card.supported_interfaces = config
            .server
            .bindings
            .iter()
            .filter_map(|b| match b.as_str() {
                "jsonrpc" => Some(a2a::AgentInterface::new(public.as_str(), "JSONRPC")),
                "rest" => Some(a2a::AgentInterface::new(public.as_str(), "HTTP+JSON")),
                "grpc" if cfg!(feature = "a2a-grpc") => {
                    Some(a2a::AgentInterface::new(public.as_str(), "GRPC"))
                }
                _ => None,
            })
            .collect();
        card.skills = registry
            .list::<Skill>()
            .iter()
            .map(|s| a2a::AgentSkill {
                id: s.entity_id().id.to_string(),
                name: s.meta().name.clone(),
                description: s.meta().description.clone(),
                tags: Vec::new(),
                examples: None,
                input_modes: None,
                output_modes: None,
                security_requirements: None,
            })
            .collect();
        let mut extensions: Vec<a2a::AgentExtension> = ext::all()
            .into_iter()
            .map(|e| a2a::AgentExtension {
                uri: e.uri,
                description: Some(format!("rusty-harness {}/v1 extension", e.name)),
                required: Some(false),
                params: Some(HashMap::from([("schema".to_owned(), serde_json::Value::String(format!("https://madarauchiha-314.github.io/rusty-harness/a2a-ext/{}.v1.schema.json", e.name)))])),
            })
            .collect();
        extensions.push(a2a::AgentExtension {
            uri: A2UI_EXT.to_owned(),
            description: Some("A2UI 0.9.1 payloads as application/a2ui+json data parts".into()),
            required: Some(false),
            params: None,
        });
        card.capabilities = a2a::AgentCapabilities {
            streaming: Some(true),
            push_notifications: Some(!config.server.push.allow_hosts.is_empty()),
            extensions: Some(extensions),
            extended_agent_card: Some(false),
        };
        match &config.server.security {
            Some(SecuritySchemeConfig::Bearer { .. }) => {
                card.security_schemes = Some(HashMap::from([(
                    "bearer".to_owned(),
                    a2a::SecurityScheme::HttpAuth(a2a::HttpAuthSecurityScheme {
                        scheme: "bearer".into(),
                        description: Some("A bearer token".into()),
                        bearer_format: None,
                    }),
                )]));
                card.security_requirements =
                    Some(vec![HashMap::from([("bearer".to_owned(), Vec::new())])]);
            }
            Some(SecuritySchemeConfig::ApiKey { header, .. }) => {
                card.security_schemes = Some(HashMap::from([(
                    "api-key".to_owned(),
                    a2a::SecurityScheme::ApiKey(a2a::ApiKeySecurityScheme {
                        location: "header".into(),
                        name: header.clone(),
                        description: Some("An API key".into()),
                    }),
                )]));
                card.security_requirements =
                    Some(vec![HashMap::from([("api-key".to_owned(), Vec::new())])]);
            }
            None => {
                card.security_schemes = None;
                card.security_requirements = None;
            }
        }
        card
    }
}

/// The participant guard on every task-scoped operation and the per-caller stream budget
/// (abuse 2, abuse 8), wrapping the SDK's handler: a caller who is not a participant
/// gets `TaskNotFound` for `GetTask`, `SubscribeToTask`, `CancelTask`, a follow-up and
/// the push-config operations, and an empty page from `ListTasks`.
pub struct GuardedHandler<H> {
    inner: H,
    store: Arc<dyn PersistenceStore>,
    gate: Arc<protocol_gap::Gate>,
}

impl<H> core::fmt::Debug for GuardedHandler<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GuardedHandler").finish_non_exhaustive()
    }
}

impl<H: RequestHandler> GuardedHandler<H> {
    /// Guards `inner`.
    #[must_use]
    pub fn new(inner: H, store: Arc<dyn PersistenceStore>, gate: Arc<protocol_gap::Gate>) -> Self {
        Self { inner, store, gate }
    }

    fn caller(params: &ServiceParams) -> Caller {
        let id = protocol_gap::caller_id(params)
            .unwrap_or_else(|| protocol_gap::DEFAULT_PRINCIPAL.to_owned());
        Caller::human(id, [])
    }

    /// `TaskNotFound` unless the caller participates in `task` (or the task does not exist,
    /// which the inner handler reports the same way).
    async fn guard(&self, params: &ServiceParams, task: &str) -> Result<(), a2a::A2AError> {
        let caller = Self::caller(params);
        match self.store.get_task(&task.to_owned()).await {
            Ok(Some(record)) if record.participants.is_empty() || caller.sees(&record) => Ok(()),
            Ok(Some(_)) => Err(a2a::A2AError::task_not_found(task)),
            Ok(None) => Ok(()),
            Err(e) => Err(a2a::A2AError::internal(e.to_string())),
        }
    }

    fn slot(&self, params: &ServiceParams) -> Result<protocol_gap::StreamSlot, a2a::A2AError> {
        let caller = Self::caller(params);
        self.gate.streams.acquire(&caller.id).ok_or_else(|| {
            a2a::A2AError::new(
                protocol_gap::error_code::TOO_MANY_STREAMS,
                "too many open streams for this caller",
            )
        })
    }

    fn hold(
        stream: BoxStream<'static, Result<a2a::StreamResponse, a2a::A2AError>>,
        slot: protocol_gap::StreamSlot,
    ) -> BoxStream<'static, Result<a2a::StreamResponse, a2a::A2AError>> {
        Box::pin(futures::stream::unfold(
            (stream, slot),
            |(mut stream, slot)| async move {
                let item = stream.next().await?;
                Some((item, (stream, slot)))
            },
        ))
    }
}

#[async_trait::async_trait]
impl<H: RequestHandler> RequestHandler for GuardedHandler<H> {
    async fn send_message(
        &self,
        params: &ServiceParams,
        req: a2a::SendMessageRequest,
    ) -> Result<a2a::SendMessageResponse, a2a::A2AError> {
        if let Some(task) = &req.message.task_id {
            self.guard(params, task).await?;
        }
        self.inner.send_message(params, req).await
    }

    async fn send_streaming_message(
        &self,
        params: &ServiceParams,
        req: a2a::SendMessageRequest,
    ) -> Result<BoxStream<'static, Result<a2a::StreamResponse, a2a::A2AError>>, a2a::A2AError> {
        if let Some(task) = &req.message.task_id {
            self.guard(params, task).await?;
        }
        let slot = self.slot(params)?;
        let stream = self.inner.send_streaming_message(params, req).await?;
        Ok(Self::hold(stream, slot))
    }

    async fn get_task(
        &self,
        params: &ServiceParams,
        req: a2a::GetTaskRequest,
    ) -> Result<a2a::Task, a2a::A2AError> {
        self.guard(params, &req.id).await?;
        let caller = Self::caller(params);
        let mut task = self.inner.get_task(params, req).await?;
        // A channel's question is for its members (R18.5): others see the state change.
        crate::app_service::channels_api::redact_for_viewer(
            self.store.as_ref(),
            &mut task,
            &caller,
        )
        .await
        .map_err(|e| a2a::A2AError::internal(e.to_string()))?;
        Ok(task)
    }

    async fn list_tasks(
        &self,
        params: &ServiceParams,
        req: a2a::ListTasksRequest,
    ) -> Result<a2a::ListTasksResponse, a2a::A2AError> {
        let caller = Self::caller(params);
        let mut page = self.inner.list_tasks(params, req).await?;
        let mut kept = Vec::with_capacity(page.tasks.len());
        for task in page.tasks {
            if let Ok(Some(record)) = self.store.get_task(&task.id).await
                && caller.sees(&record)
            {
                kept.push(task);
            }
        }
        page.total_size = i32::try_from(kept.len()).unwrap_or(i32::MAX);
        page.page_size = page.total_size;
        page.tasks = kept;
        Ok(page)
    }

    async fn cancel_task(
        &self,
        params: &ServiceParams,
        req: a2a::CancelTaskRequest,
    ) -> Result<a2a::Task, a2a::A2AError> {
        self.guard(params, &req.id).await?;
        self.inner.cancel_task(params, req).await
    }

    async fn subscribe_to_task(
        &self,
        params: &ServiceParams,
        req: a2a::SubscribeToTaskRequest,
    ) -> Result<BoxStream<'static, Result<a2a::StreamResponse, a2a::A2AError>>, a2a::A2AError> {
        self.guard(params, &req.id).await?;
        let slot = self.slot(params)?;
        // Served from the harness's log, not the SDK's execution manager: a waiting task
        // has no execution there, but its log is the truth (design.md § A2A API server).
        let caller = Self::caller(params);
        let viewer =
            crate::app_service::channels_api::viewer_for(self.store.as_ref(), &req.id, &caller)
                .await
                .map_err(|e| a2a::A2AError::internal(e.to_string()))?;
        if self
            .store
            .get_task(&req.id)
            .await
            .map_err(|e| a2a::A2AError::internal(e.to_string()))?
            .is_none()
        {
            return Err(a2a::A2AError::task_not_found(&req.id));
        }
        let stream = log_stream(Arc::clone(&self.store), req.id.clone(), 1, viewer).await;
        Ok(Self::hold(stream, slot))
    }

    async fn create_push_config(
        &self,
        params: &ServiceParams,
        req: a2a::TaskPushNotificationConfig,
    ) -> Result<a2a::TaskPushNotificationConfig, a2a::A2AError> {
        self.guard(params, &req.task_id).await?;
        self.inner.create_push_config(params, req).await
    }

    async fn get_push_config(
        &self,
        params: &ServiceParams,
        req: a2a::GetTaskPushNotificationConfigRequest,
    ) -> Result<a2a::TaskPushNotificationConfig, a2a::A2AError> {
        self.guard(params, &req.task_id).await?;
        self.inner.get_push_config(params, req).await
    }

    async fn list_push_configs(
        &self,
        params: &ServiceParams,
        req: a2a::ListTaskPushNotificationConfigsRequest,
    ) -> Result<a2a::ListTaskPushNotificationConfigsResponse, a2a::A2AError> {
        self.guard(params, &req.task_id).await?;
        self.inner.list_push_configs(params, req).await
    }

    async fn delete_push_config(
        &self,
        params: &ServiceParams,
        req: a2a::DeleteTaskPushNotificationConfigRequest,
    ) -> Result<(), a2a::A2AError> {
        self.guard(params, &req.task_id).await?;
        self.inner.delete_push_config(params, req).await
    }

    async fn get_extended_agent_card(
        &self,
        params: &ServiceParams,
        req: a2a::GetExtendedAgentCardRequest,
    ) -> Result<a2a::AgentCard, a2a::A2AError> {
        self.inner.get_extended_agent_card(params, req).await
    }
}

/// The router: JSON-RPC at `/`, HTTP+JSON at `/v1/…`, the card at its well-known path.
///
/// # Errors
///
/// A credential that does not resolve.
pub async fn router(harness: &Harness) -> Result<axum::Router, crate::HarnessError> {
    router_at(harness, &harness.config().server.bind).await
}

/// The router for a server bound at `bound`, behind the gate of `protocol_gap`.
///
/// # Errors
///
/// A credential that does not resolve.
pub async fn router_at(
    harness: &Harness,
    bound: &str,
) -> Result<axum::Router, crate::HarnessError> {
    let agent = Arc::clone(harness.agent());
    let card =
        AgentCardBuilder::build_at(harness.config(), harness.registry(), agent.card(), bound);
    let advertised: Vec<String> = card
        .capabilities
        .extensions
        .iter()
        .flatten()
        .map(|e| e.uri.clone())
        .collect();
    let gate = Arc::new(
        protocol_gap::Gate::new(
            harness.config().server.security.as_ref(),
            &harness.config().server.limits,
            advertised,
            harness.host(),
        )
        .await?,
    );
    let inbox = harness.inbox().cloned().ok_or_else(|| {
        crate::HarnessError::Internal("the server needs the in-process inbox (rt-tokio)".into())
    })?;
    let handler = a2a_server::DefaultRequestHandler::new(
        HarnessExecutor::new(Arc::clone(&agent), Arc::clone(&inbox)),
        SdkTaskStore::new(Arc::clone(&agent)),
    )
    .with_capabilities(card.capabilities.clone())
    .with_default_input_modes(card.default_input_modes.clone())
    .with_push_notifications(
        AllowlistedPushStore::new(harness.config().server.push.allow_hosts.clone()),
        a2a_server::HttpPushSender::new(None),
    );
    let handler = Arc::new(GuardedHandler::new(
        handler,
        Arc::clone(agent.store()),
        Arc::clone(&gate),
    ));
    let mut app = axum::Router::new().merge(a2a_server::agent_card::agent_card_router(Arc::new(
        StaticAgentCard::new(card),
    )));
    for binding in &harness.config().server.bindings {
        match binding.as_str() {
            "jsonrpc" => app = app.merge(a2a_server::jsonrpc::jsonrpc_router(Arc::clone(&handler))),
            "rest" => app = app.merge(a2a_server::rest::rest_router(Arc::clone(&handler))),
            _ => {}
        }
    }
    // `POST /inbox`: the second route into the inbox (R20.1–2), behind the same gate.
    app = app.merge(
        axum::Router::new()
            .route("/inbox", axum::routing::post(post_inbox))
            .with_state(InboxRoute {
                inbox,
                store: Arc::clone(agent.store()),
                host: Arc::clone(agent.host()),
            }),
    );
    Ok(app.layer(axum::middleware::from_fn_with_state(
        gate,
        protocol_gap::gate,
    )))
}

/// What the inbox route needs.
#[derive(Clone)]
struct InboxRoute {
    inbox: Arc<crate::app_service::inbox::Inbox>,
    store: Arc<dyn PersistenceStore>,
    host: Arc<crate::host::Host>,
}

/// `POST /inbox`: one A2A event addressed to a task or a context (an [`InboxPost`]),
/// accepted through the same gate, the same participant guard and the same
/// `Inbox::accept` as a `SendMessage` (R20.1–2, boundary 2). `202` with the event's
/// reference; `404` for a task the caller does not participate in, as for any other
/// operation (abuse 2); `409` for a terminal task; `400` for a body that is not an event.
///
/// [`InboxPost`]: crate::interaction::events::InboxPost
#[allow(clippy::too_many_lines)] // one request, read top to bottom
async fn post_inbox(
    axum::extract::State(route): axum::extract::State<InboxRoute>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use crate::app_service::inbox::{
        DeliveryKind, EventTarget, InboxError, InboxEvent, InboxStatus,
    };
    use crate::interaction::events::{InboxPostAdapter, WebhookAdapter as _};
    use axum::response::IntoResponse as _;

    let lower: std::collections::BTreeMap<String, String> = headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_ascii_lowercase(),
                v.to_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    let params: ServiceParams = lower
        .iter()
        .map(|(k, v)| (k.clone(), vec![v.clone()]))
        .collect();
    let error = |status: http::StatusCode, err: a2a::A2AError| {
        (
            status,
            axum::Json(serde_json::json!({ "code": err.code, "message": err.message })),
        )
            .into_response()
    };
    let post = match InboxPostAdapter.translate(&lower, &body) {
        Ok(Some(post)) => post,
        Ok(None) => {
            return error(
                http::StatusCode::BAD_REQUEST,
                a2a::A2AError::invalid_params("not an event"),
            );
        }
        Err(e) => {
            return error(
                http::StatusCode::BAD_REQUEST,
                a2a::A2AError::invalid_params(e.to_string()),
            );
        }
    };
    let message_task = match &post.event {
        a2a::StreamResponse::Message(m) => m.task_id.clone(),
        _ => None,
    };
    let target = match (post.task_id.or(message_task), post.context_id, &post.event) {
        (Some(task), _, _) => EventTarget::Task(task),
        (None, Some(context), a2a::StreamResponse::Message(_)) => EventTarget::Context(context),
        _ => {
            return error(
                http::StatusCode::BAD_REQUEST,
                a2a::A2AError::invalid_params("an event must name a task, or a message a context"),
            );
        }
    };
    let caller = Caller::human(
        protocol_gap::caller_id(&params)
            .unwrap_or_else(|| protocol_gap::DEFAULT_PRINCIPAL.to_owned()),
        [],
    );
    // The participant guard (abuse 2): a stranger learns nothing, not even that the
    // task exists.
    if let EventTarget::Task(task) = &target
        && let Ok(Some(record)) = route.store.get_task(task).await
        && !record.participants.is_empty()
        && !caller.sees(&record)
    {
        return error(
            http::StatusCode::NOT_FOUND,
            a2a::A2AError::task_not_found(task),
        );
    }
    let delivery = match &post.event {
        a2a::StreamResponse::Message(_) => DeliveryKind::Interrupt,
        _ => DeliveryKind::Queue,
    };
    let id = match &post.event {
        a2a::StreamResponse::Message(m) if !m.message_id.is_empty() => m.message_id.clone(),
        _ => a2a::new_message_id(),
    };
    let event = InboxEvent {
        id,
        target,
        kind: post.event,
        status: InboxStatus::Received,
        sender: caller,
        received_at: route
            .host
            .clock()
            .map_or_else(|_| chrono::Utc::now(), |c| c.now()),
        delivery,
        extensions: if post.extensions.is_empty() {
            protocol_gap::active_extensions(&params)
        } else {
            post.extensions
        },
        traceparent: crate::app_service::o11y::trace::incoming(
            lower
                .get(crate::app_service::o11y::trace::HEADER)
                .map(String::as_str),
        ),
    };
    match route.inbox.accept(event).await {
        Ok(reference) => (
            http::StatusCode::ACCEPTED,
            axum::Json(serde_json::to_value(&reference).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => {
            let status = match &e {
                InboxError::NotFound(_) => http::StatusCode::NOT_FOUND,
                InboxError::Terminal(_) => http::StatusCode::CONFLICT,
                InboxError::ContextMismatch { .. } | InboxError::Invalid(_) => {
                    http::StatusCode::BAD_REQUEST
                }
                _ => http::StatusCode::INTERNAL_SERVER_ERROR,
            };
            error(status, e.to_a2a())
        }
    }
}

/// Why the server refused to start.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// A non-loopback bind without a security scheme (the fail-closed list).
    #[error("bind {0} is not a loopback address and [server.security] is not set")]
    LoopbackOnly(String),
    /// A credential that does not resolve.
    #[error(transparent)]
    Harness(#[from] crate::HarnessError),
    /// The socket.
    #[error("bind {bind}: {source}")]
    Bind {
        /// The address.
        bind: String,
        /// The error.
        source: std::io::Error,
    },
}

/// A bound server: its address and the future that serves it.
pub struct Server {
    /// Where it listens.
    pub addr: std::net::SocketAddr,
    listener: tokio::net::TcpListener,
    app: axum::Router,
}

impl core::fmt::Debug for Server {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Server")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Serves until the process ends (or `shutdown` resolves).
    ///
    /// # Errors
    ///
    /// The accept loop's error.
    pub async fn run(
        self,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        axum::serve(self.listener, self.app)
            .with_graceful_shutdown(shutdown)
            .await
    }
}

/// Binds `[server] bind` (or `bind` when given) for `harness` (R19.1); without a
/// security scheme only a loopback address is accepted (fail-closed list).
///
/// # Errors
///
/// [`ServeError`].
pub async fn serve(harness: &Harness, bind: Option<&str>) -> Result<Server, ServeError> {
    let bind = bind.map_or_else(|| harness.config().server.bind.clone(), str::to_owned);
    let addr: std::net::SocketAddr =
        bind.parse()
            .map_err(|e: std::net::AddrParseError| ServeError::Bind {
                bind: bind.clone(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidInput, e),
            })?;
    if harness.config().server.security.is_none() && !addr.ip().is_loopback() {
        return Err(ServeError::LoopbackOnly(bind));
    }
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| ServeError::Bind {
            bind: bind.clone(),
            source,
        })?;
    let addr = listener
        .local_addr()
        .map_err(|source| ServeError::Bind { bind, source })?;
    Ok(Server {
        addr,
        listener,
        app: router_at(harness, &addr.to_string()).await?,
    })
}

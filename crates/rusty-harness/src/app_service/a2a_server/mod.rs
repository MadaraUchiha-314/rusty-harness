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

use a2a_server::{AgentExecutor, ExecutorContext, StaticAgentCard, TaskStore};
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
}

impl core::fmt::Debug for HarnessExecutor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HarnessExecutor").finish_non_exhaustive()
    }
}

impl HarnessExecutor {
    /// The executor over `agent`.
    #[must_use]
    pub fn new(agent: Arc<LocalAgent>) -> Self {
        Self { agent }
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
        let caller = Self::caller(&ctx);
        let Some(message) = ctx.message else {
            return Box::pin(futures::stream::iter(vec![Err(
                a2a::A2AError::invalid_params("execute without a message"),
            )]));
        };
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
        };
        let task = ctx.task_id;
        Box::pin(
            futures::stream::once(async move {
                let store = Arc::clone(agent.store());
                let from = store
                    .events(&task, 1, usize::MAX, &caller)
                    .await
                    .map_or(1, |e| e.len() as u64 + 1);
                let opened = agent.open_task(&request).await;
                let run = match opened {
                    Ok((_, true)) => {
                        let agent = Arc::clone(&agent);
                        let task = task.clone();
                        tokio::spawn(async move {
                            let _ = agent.run(&task).await;
                        });
                        true
                    }
                    Ok((_, false)) => false,
                    Err(e) => return futures::stream::iter(vec![Err(e.to_a2a())]).boxed(),
                };
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
            push_notifications: Some(false),
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

/// The router: JSON-RPC at `/`, HTTP+JSON at `/v1/…`, the card at its well-known path.
pub fn router(harness: &Harness) -> axum::Router {
    router_at(harness, &harness.config().server.bind)
}

/// The router for a server bound at `bound`.
pub fn router_at(harness: &Harness, bound: &str) -> axum::Router {
    let agent = Arc::clone(harness.agent());
    let card =
        AgentCardBuilder::build_at(harness.config(), harness.registry(), agent.card(), bound);
    let handler = a2a_server::DefaultRequestHandler::new(
        HarnessExecutor::new(Arc::clone(&agent)),
        SdkTaskStore::new(agent),
    )
    .with_capabilities(card.capabilities.clone())
    .with_default_input_modes(card.default_input_modes.clone());
    let handler = Arc::new(handler);
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
    app
}

/// Why the server refused to start.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// A non-loopback bind without a security scheme (the fail-closed list).
    #[error("bind {0} is not a loopback address and [server.security] is not set")]
    LoopbackOnly(String),
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
        app: router_at(harness, &addr.to_string()),
    })
}

//! The SDK door: [`HarnessBuilder`] assembles what the CLI assembles, from the same
//! `Config`, and hands back a [`Harness`] usable in process with no service layer
//! (R30.1, R11.6; design.md § Packaging and SDK parity).

use std::sync::Arc;

use crate::assets::PromptRegistry;
use crate::config::{Config, DurableMode, ModelProvider};
use crate::harness::a2a_client::AgentCallTool;
use crate::harness::channels::{ParticipantAskTool, ParticipantDelegateTool};
use crate::harness::commands::{ModelRouter, ModelSelection, install_builtins};
use crate::harness::core_loop::agent::{LocalAgent, LocalAgentParts};
use crate::harness::core_loop::persistence::{MemoryStore, PersistenceStore};
use crate::harness::hooks::{Hook, HookDispatcher, HookOrder};
use crate::harness::models::llm::Llm;
use crate::harness::plugins::{InstalledPlugin, McpServerEntity, PluginError, PluginLoader};
use crate::harness::tools::{McpServer, Tool, ToolError, ToolExecute, ToolPolicy};
use crate::host::Host;
use crate::registry::{Entity, EntityId, Registry};

/// Why a harness could not be built; every variant is fail-closed (nothing half-built
/// is returned).
#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    /// A model under `[models]` could not be constructed.
    #[error("model {alias}: {reason}")]
    Model {
        /// The alias.
        alias: String,
        /// Why.
        reason: String,
    },
    /// No `default` model: neither configured nor handed to the builder.
    #[error("no `default` model: configure [models.default] or pass one to the builder")]
    NoDefaultModel,
    /// A plugin was refused.
    #[error(transparent)]
    Plugin(#[from] PluginError),
    /// An MCP server could not be connected.
    #[error("mcp server {server}: {source}")]
    Mcp {
        /// The server.
        server: String,
        /// Why.
        source: ToolError,
    },
    /// A sub-agent's card was refused.
    #[error("sub-agent {name}: {reason}")]
    SubAgent {
        /// The alias under `[agents]`.
        name: String,
        /// Why.
        reason: String,
    },
    /// The store could not be opened.
    #[error("store: {0}")]
    Store(String),
    /// A registry conflict between the pieces.
    #[error("registry: {0}")]
    Registry(String),
    /// A durable mode this build cannot run.
    #[error("durable mode {0:?} needs the `durable-temporal` feature and the service layer")]
    Durable(DurableMode),
}

/// Builds a [`Harness`]: `HarnessBuilder::new(config).host(host).llm(..).build().await`.
pub struct HarnessBuilder {
    config: Config,
    host: Option<Host>,
    llms: Vec<Arc<dyn Llm>>,
    tools: Vec<Arc<dyn Tool>>,
    store: Option<Arc<dyn PersistenceStore>>,
    registry: Option<Arc<Registry>>,
    dispatcher: Option<Arc<HookDispatcher>>,
    prompts: Option<PromptRegistry>,
    tracing: bool,
}

impl core::fmt::Debug for HarnessBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HarnessBuilder")
            .field("agent", &self.config.agent.name)
            .field("llms", &self.llms.len())
            .field("tools", &self.tools.len())
            .finish_non_exhaustive()
    }
}

impl HarnessBuilder {
    /// A builder for `config`.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            host: None,
            llms: Vec::new(),
            tools: Vec::new(),
            store: None,
            registry: None,
            dispatcher: None,
            prompts: None,
            tracing: true,
        }
    }

    /// The platform. Natively the default is [`Host::native`]; in the browser the host
    /// must be supplied.
    #[must_use]
    pub fn host(mut self, host: Host) -> Self {
        self.host = Some(host);
        self
    }

    /// A model registered under its own alias, besides (or instead of) `[models]`.
    #[must_use]
    pub fn llm(mut self, llm: Arc<dyn Llm>) -> Self {
        self.llms.push(llm);
        self
    }

    /// A tool registered besides the built-ins and the plugins'.
    #[must_use]
    pub fn tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.push(tool);
        self
    }

    /// A store, replacing `[persistence]`.
    #[must_use]
    pub fn store(mut self, store: Arc<dyn PersistenceStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// A registry to build into (yours, with entities already registered).
    #[must_use]
    pub fn registry(mut self, registry: Arc<Registry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// A dispatcher to build onto (yours, with hooks already registered).
    #[must_use]
    pub fn hooks(mut self, dispatcher: Arc<HookDispatcher>) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    /// The prompt registry to start from (default: the built-in fragments).
    #[must_use]
    pub fn prompts(mut self, prompts: PromptRegistry) -> Self {
        self.prompts = Some(prompts);
        self
    }

    /// Whether to install the tracing hooks on every operation (default: yes).
    #[must_use]
    pub fn tracing(mut self, on: bool) -> Self {
        self.tracing = on;
        self
    }

    /// Assembles the harness (R30.1). Fails closed: a bad model, plugin, server or card
    /// returns an error and nothing is left behind in a shared registry beyond what was
    /// registered before the failure.
    ///
    /// # Errors
    ///
    /// [`SdkError`].
    #[allow(clippy::too_many_lines)]
    pub async fn build(self) -> Result<Harness, SdkError> {
        let Self {
            config,
            host,
            llms,
            tools,
            store,
            registry,
            dispatcher,
            prompts,
            tracing,
        } = self;
        #[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
        let host = Arc::new(host.unwrap_or_else(Host::native));
        #[cfg(not(all(feature = "rt-tokio", not(target_arch = "wasm32"))))]
        let host = Arc::new(host.unwrap_or_else(|| Host::builder().build()));
        let registry = registry.unwrap_or_else(|| Arc::new(Registry::new()));
        let dispatcher = dispatcher.unwrap_or_else(|| Arc::new(HookDispatcher::new()));
        let mut prompts = prompts.unwrap_or_else(PromptRegistry::builtin);
        let reg = |e: crate::registry::RegistryError| SdkError::Registry(e.to_string());

        // Models (R9): configured ones, then the ones handed in.
        for (alias, model) in &config.models {
            let llm: Arc<dyn Llm> = match model.provider {
                ModelProvider::Openai => Arc::new(
                    crate::harness::models::llm::openai::OpenAiLlm::from_config(
                        alias, model, &host,
                    )
                    .await
                    .map_err(|e| SdkError::Model {
                        alias: alias.clone(),
                        reason: e.to_string(),
                    })?,
                ),
                ModelProvider::Anthropic => Arc::new(
                    crate::harness::models::llm::anthropic::AnthropicLlm::from_config(
                        alias, model, &host,
                    )
                    .await
                    .map_err(|e| SdkError::Model {
                        alias: alias.clone(),
                        reason: e.to_string(),
                    })?,
                ),
            };
            registry.register(llm).map_err(reg)?;
        }
        for llm in llms {
            registry.register(llm).map_err(reg)?;
        }
        if registry.latest::<dyn Llm>("default").is_err() {
            return Err(SdkError::NoDefaultModel);
        }
        let router = Arc::new(ModelRouter::new(
            Arc::clone(&registry),
            ModelSelection {
                model: "default".into(),
                effort: None,
            },
        ));

        // Plugins (R5), their MCP servers (R8.1).
        let mut plugins = Vec::new();
        if !config.plugins.paths.is_empty() {
            let fs = host.fs().map_err(|e| {
                SdkError::Plugin(PluginError::Invalid {
                    plugin: "<plugins>".into(),
                    field: "paths".into(),
                    reason: e.to_string(),
                })
            })?;
            let data_dir = config
                .persistence
                .path
                .clone()
                .unwrap_or_else(|| std::path::PathBuf::from(".rusty-harness"));
            let loader = PluginLoader::new(data_dir);
            let (specs, errors) = loader.discover(&config.plugins.paths, fs.as_ref()).await;
            if let Some(error) = errors.into_iter().next() {
                return Err(error.into());
            }
            for spec in &specs {
                plugins.push(loader.install(spec, &registry, &mut prompts)?);
            }
        }
        for entity in registry.list::<McpServerEntity>() {
            let server = McpServer::connect(entity.entity_id().clone(), &entity.spec, &host)
                .await
                .map_err(|source| SdkError::Mcp {
                    server: entity.entity_id().id.to_string(),
                    source,
                })?;
            server
                .register_tools(&registry)
                .await
                .map_err(|source| SdkError::Mcp {
                    server: entity.entity_id().id.to_string(),
                    source,
                })?;
            registry.register(Arc::new(server)).map_err(reg)?;
        }

        // Store (R14.2).
        let store: Arc<dyn PersistenceStore> = match store {
            Some(s) => s,
            None => open_store(&config).await?,
        };

        // Built-in tools and commands (R17.3, R18.4, R10.2), the tool policy guard.
        let built_ins: Vec<Arc<dyn Tool>> = vec![
            Arc::new(AgentCallTool::new(
                Arc::clone(&registry),
                Arc::clone(&dispatcher),
                Arc::clone(&store),
            )),
            Arc::new(ParticipantAskTool::new(
                Arc::clone(&store),
                Arc::clone(&dispatcher),
            )),
            Arc::new(ParticipantDelegateTool::new(
                Arc::clone(&store),
                Arc::clone(&dispatcher),
                router.selection(),
            )),
        ];
        for tool in built_ins.into_iter().chain(tools) {
            registry.register(tool).map_err(reg)?;
        }
        install_builtins(&registry, &dispatcher, &router)
            .map_err(|e| SdkError::Registry(e.to_string()))?;
        dispatcher.register::<ToolExecute>(
            EntityId::new("tools.policy"),
            Hook::Guard(Arc::new(ToolPolicy::default())),
            HookOrder::default(),
        );
        if tracing {
            install_tracing(&dispatcher);
        }

        // Sub-agents (R17.1).
        #[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
        for (name, sub) in &config.agents {
            let agent: Arc<dyn crate::harness::core_loop::agent::Agent> =
                Arc::new(connect_sub_agent(name, sub, &host).await?);
            registry.register(agent).map_err(reg)?;
        }

        let prompts = Arc::new(prompts);
        let agent = Arc::new(LocalAgent::new(LocalAgentParts {
            config: config.clone(),
            registry: Arc::clone(&registry),
            dispatcher: Arc::clone(&dispatcher),
            host: Arc::clone(&host),
            store: Arc::clone(&store),
            prompts: Arc::clone(&prompts),
            router,
        }));
        // The harness as its own sub-agent (R15.3).
        let loopback =
            crate::harness::a2a_client::loopback_agent(&agent).map_err(|e| SdkError::SubAgent {
                name: "loopback".into(),
                reason: e.to_string(),
            })?;
        let loopback: Arc<dyn crate::harness::core_loop::agent::Agent> = Arc::new(loopback);
        registry.register(loopback).map_err(reg)?;

        if config.durable.mode != DurableMode::None {
            return Err(SdkError::Durable(config.durable.mode));
        }
        #[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
        let durable: Option<Arc<dyn crate::app_service::durable::DurableExecution>> = Some(
            Arc::new(crate::app_service::durable::none::InProcessExecution::new(
                Arc::clone(&agent),
                Arc::new(crate::runtime::TokioRuntime::current()),
                crate::app_service::durable::RetryPolicy::from(&config.durable.retry),
            )),
        );
        #[cfg(not(all(feature = "rt-tokio", not(target_arch = "wasm32"))))]
        let durable: Option<Arc<dyn crate::app_service::durable::DurableExecution>> = None;
        let inbox = durable.as_ref().map(|d| {
            Arc::new(crate::app_service::inbox::Inbox::new(
                Arc::clone(&agent),
                Arc::clone(d),
                Arc::clone(&dispatcher),
            ))
        });
        let heartbeat = durable.as_ref().map(|d| {
            Arc::new(crate::app_service::heartbeat::Heartbeat::new(
                format!("{}-{}", config.agent.name, std::process::id()),
                &config.r#loop,
                Arc::clone(&agent),
                Arc::clone(d),
                Arc::clone(&dispatcher),
            ))
        });
        Ok(Harness {
            config,
            host,
            registry,
            dispatcher,
            store,
            prompts,
            agent,
            plugins,
            durable,
            inbox,
            heartbeat,
        })
    }
}

async fn open_store(config: &Config) -> Result<Arc<dyn PersistenceStore>, SdkError> {
    match config.persistence.store.as_str() {
        "memory" => Ok(Arc::new(MemoryStore::new())),
        #[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
        "file" => {
            let path = config.persistence.path.clone().ok_or_else(|| {
                SdkError::Store("[persistence] path is required for the file store".into())
            })?;
            Ok(Arc::new(
                crate::harness::core_loop::persistence::FileStore::open(path)
                    .await
                    .map_err(|e| SdkError::Store(e.to_string()))?,
            ))
        }
        other => Err(SdkError::Store(format!("unknown store {other}"))),
    }
}

#[cfg(all(feature = "rt-tokio", not(target_arch = "wasm32")))]
async fn connect_sub_agent(
    name: &str,
    sub: &crate::config::SubAgentConfig,
    host: &Host,
) -> Result<crate::harness::a2a_client::RemoteAgent, SdkError> {
    use crate::harness::a2a_client::official::{OfficialTransport, fetch_card};
    let err = |reason: String| SdkError::SubAgent {
        name: name.to_owned(),
        reason,
    };
    let card = fetch_card(host, &sub.card_url)
        .await
        .map_err(|e| err(e.to_string()))?;
    let bearer = match &sub.api_key {
        Some(reference) => {
            let secrets = host.secrets().map_err(|e| err(e.to_string()))?;
            Some(
                secrets
                    .resolve(reference)
                    .await
                    .map_err(|e| err(e.to_string()))?,
            )
        }
        None => None,
    };
    let transport =
        OfficialTransport::from_card(&card, bearer, crate::harness::core_loop::task::ext::uris())
            .map_err(|e| err(e.to_string()))?;
    crate::harness::a2a_client::RemoteAgent::new(EntityId::new(name), card, Arc::new(transport))
        .map_err(|e| err(e.to_string()))
}

/// Installs the tracing span hook on every operation the harness dispatches (R24).
fn install_tracing(dispatcher: &HookDispatcher) {
    use crate::app_service::o11y::TracingHooks;
    use crate::harness::core_loop::agent::{ModelCall, ResponseDeliver, ResponseParse, TaskRun};
    TracingHooks::install::<TaskRun>(dispatcher);
    TracingHooks::install::<ModelCall>(dispatcher);
    TracingHooks::install::<ResponseParse>(dispatcher);
    TracingHooks::install::<ResponseDeliver>(dispatcher);
    TracingHooks::install::<ToolExecute>(dispatcher);
    TracingHooks::install::<crate::harness::tools::ToolResultSize>(dispatcher);
    TracingHooks::install::<crate::harness::core_loop::context_window::ContextBuild>(dispatcher);
    TracingHooks::install::<crate::harness::core_loop::compaction::ContextCompact>(dispatcher);
    TracingHooks::install::<crate::harness::core_loop::task::TaskCreate>(dispatcher);
    TracingHooks::install::<crate::harness::core_loop::task::TaskStateChange>(dispatcher);
    TracingHooks::install::<crate::harness::core_loop::plan::PlanPersist>(dispatcher);
    TracingHooks::install::<crate::harness::core_loop::state::StatePersist>(dispatcher);
    TracingHooks::install::<crate::harness::system_prompt::PromptAssemble>(dispatcher);
    TracingHooks::install::<crate::harness::commands::CommandInvoke>(dispatcher);
    TracingHooks::install::<crate::harness::a2a_client::A2aSend>(dispatcher);
    TracingHooks::install::<crate::harness::channels::ParticipantAsk>(dispatcher);
    TracingHooks::install::<crate::harness::channels::ParticipantDelegate>(dispatcher);
    TracingHooks::install::<crate::harness::skills::SkillInvoke>(dispatcher);
}

/// A built harness: the agent and the pieces behind it.
pub struct Harness {
    config: Config,
    host: Arc<Host>,
    registry: Arc<Registry>,
    dispatcher: Arc<HookDispatcher>,
    store: Arc<dyn PersistenceStore>,
    prompts: Arc<PromptRegistry>,
    agent: Arc<LocalAgent>,
    plugins: Vec<InstalledPlugin>,
    durable: Option<Arc<dyn crate::app_service::durable::DurableExecution>>,
    inbox: Option<Arc<crate::app_service::inbox::Inbox>>,
    heartbeat: Option<Arc<crate::app_service::heartbeat::Heartbeat>>,
}

impl core::fmt::Debug for Harness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Harness")
            .field("agent", &self.config.agent.name)
            .field("plugins", &self.plugins.len())
            .finish_non_exhaustive()
    }
}

impl Harness {
    /// The agent: `harness.agent().send_message(req)` runs a task in process.
    #[must_use]
    pub fn agent(&self) -> &Arc<LocalAgent> {
        &self.agent
    }

    /// The configuration the harness was built from.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The platform.
    #[must_use]
    pub fn host(&self) -> &Arc<Host> {
        &self.host
    }

    /// Every entity.
    #[must_use]
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Every hook.
    #[must_use]
    pub fn hooks(&self) -> &Arc<HookDispatcher> {
        &self.dispatcher
    }

    /// The store.
    #[must_use]
    pub fn store(&self) -> &Arc<dyn PersistenceStore> {
        &self.store
    }

    /// The prompt fragments, built-ins and plugins' together.
    #[must_use]
    pub fn prompts(&self) -> &Arc<PromptRegistry> {
        &self.prompts
    }

    /// What the plugins installed.
    #[must_use]
    pub fn plugins(&self) -> &[InstalledPlugin] {
        &self.plugins
    }

    /// Durable execution over this harness's agent (mode `none` in process): resumes open
    /// tasks after a restart (R14.4) and signals running ones. `None` where no runtime
    /// can spawn (the browser build drives the loop itself).
    #[must_use]
    pub fn durable(&self) -> Option<&Arc<dyn crate::app_service::durable::DurableExecution>> {
        self.durable.as_ref()
    }

    /// The durable inbox: the one ingestion path for every event (R20.2).
    #[must_use]
    pub fn inbox(&self) -> Option<&Arc<crate::app_service::inbox::Inbox>> {
        self.inbox.as_ref()
    }

    /// The heartbeat (R23); the server starts it, an embedder may tick it.
    #[must_use]
    pub fn heartbeat(&self) -> Option<&Arc<crate::app_service::heartbeat::Heartbeat>> {
        self.heartbeat.as_ref()
    }
}

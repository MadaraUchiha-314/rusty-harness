//! Typed configuration: the `Config` tree, the TOML loader with defaults ← file ←
//! environment ← code layering, and the generated JSON schema (R27.2, R30.4, Q9).
//!
//! One `Config` is what the TOML file, the environment and the SDKs all produce. The
//! loader works on `toml::Value` tables: parse the file, overlay the environment
//! (`RH__MODELS__DEFAULT__MODEL=gpt-6.1` becomes `models.default.model`), overlay the
//! code overrides, deserialize once with unknown fields refused. Credentials appear only
//! as [`SecretRef`]s; a literal in their place fails to parse without being echoed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::host::{FileSystem, HostError, SecretRef};

mod duration;

pub use duration::{DurationStr, InvalidDuration};

/// The commented default configuration the first run writes (R27.2); the same file the
/// `snapshot_default_config_toml` test keeps equal to [`Config::default`].
pub const DEFAULT_CONFIG_TOML: &str = include_str!("../../config.default.toml");

/// The whole configuration (design.md § Data models § Configuration).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The agent this harness serves.
    pub agent: AgentConfig,
    /// Language models by alias; `default` is the one the loop starts with.
    pub models: BTreeMap<String, ModelConfig>,
    /// System One models by alias.
    pub system_one: BTreeMap<String, SystemOneConfig>,
    /// The A2A server.
    pub server: ServerConfig,
    /// Durable execution.
    pub durable: DurableConfig,
    /// The persistence store.
    pub persistence: PersistenceConfig,
    /// Plugin discovery.
    pub plugins: PluginsConfig,
    /// Sub-agents by name.
    pub agents: BTreeMap<String, SubAgentConfig>,
    /// Hook ordering and policies.
    pub hooks: HooksConfig,
    /// Tool execution.
    pub tools: ToolsConfig,
    /// The context window.
    pub context: ContextConfig,
    /// The core loop's limits.
    pub r#loop: LoopConfig,
    /// Voice.
    pub voice: VoiceConfig,
    /// Observability.
    pub o11y: O11yConfig,
    /// The terminal UI.
    pub tui: TuiConfig,
    /// The locale for user-facing strings (R33.8); `LANG` then `en` when absent.
    pub locale: Option<String>,
}

/// `[agent]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    /// The agent's name, as the agent card shows it.
    pub name: String,
    /// One sentence for the agent card.
    pub description: String,
    /// A prompt fragment reference (`file:` or a registered id) replacing the default
    /// system prompt (R7.2).
    pub system_prompt: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: "rusty-harness".into(),
            description: "An agent served by rusty-harness.".into(),
            system_prompt: None,
        }
    }
}

/// A language model provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ModelProvider {
    /// `OpenAI`, or anything speaking its API through `base_url` (`OpenRouter`, Ollama).
    Openai,
    /// Anthropic.
    Anthropic,
}

/// `[models.<alias>]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    /// The provider.
    pub provider: ModelProvider,
    /// The model id the provider knows.
    pub model: String,
    /// The credential, by reference.
    pub api_key: SecretRef,
    /// The API base URL for `OpenAI`-compatible providers; the provider's own when absent.
    #[serde(default)]
    pub base_url: Option<url::Url>,
    /// The default reasoning effort (`low`, `medium`, `high`).
    #[serde(default)]
    pub effort: Option<String>,
    /// Retry policy for retryable provider errors (R9.6).
    #[serde(default)]
    pub retry: RetryConfig,
}

/// `[system_one.<alias>]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SystemOneConfig {
    /// The provider; `http` is the generic typed classifier endpoint (Q4).
    pub provider: String,
    /// The endpoint.
    pub url: url::Url,
    /// The credential, by reference.
    #[serde(default)]
    pub api_key: Option<SecretRef>,
}

/// A retry policy with exponential backoff (R9.6, R22.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RetryConfig {
    /// The first wait.
    pub initial: DurationStr,
    /// Multiplier per attempt.
    pub coefficient: u32,
    /// The longest wait.
    pub max_interval: DurationStr,
    /// Attempts in total; always finite.
    pub max_attempts: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            initial: DurationStr::secs(1),
            coefficient: 2,
            max_interval: DurationStr::secs(100),
            max_attempts: 5,
        }
    }
}

/// `[server]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// The socket to bind. Without a security scheme only a loopback address is accepted.
    pub bind: String,
    /// The bindings to serve: `jsonrpc`, `rest` and, with the feature, `grpc`.
    pub bindings: Vec<String>,
    /// The URL clients reach the server at, for the agent card.
    pub public_url: Option<url::Url>,
    /// Authentication (R19.4, Q10); absent means loopback only.
    pub security: Option<SecuritySchemeConfig>,
    /// Per-caller limits (abuse 8).
    pub limits: LimitsConfig,
    /// Push notifications (abuse 9).
    pub push: PushConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:7474".into(),
            bindings: vec!["jsonrpc".into(), "rest".into()],
            public_url: None,
            security: None,
            limits: LimitsConfig::default(),
            push: PushConfig::default(),
        }
    }
}

/// `[server.security]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "scheme", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SecuritySchemeConfig {
    /// A bearer token every caller must present.
    Bearer {
        /// The token, by reference.
        token: SecretRef,
    },
    /// An API key in a header.
    ApiKey {
        /// The header name.
        header: String,
        /// The key, by reference.
        key: SecretRef,
    },
}

/// `[server.limits]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LimitsConfig {
    /// Requests per minute per caller.
    pub requests_per_minute: u32,
    /// Open streams per caller.
    pub streams_per_caller: u32,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: 120,
            streams_per_caller: 8,
        }
    }
}

/// `[server.push]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PushConfig {
    /// Hosts a push-notification webhook may name; empty refuses every push config.
    pub allow_hosts: Vec<String>,
}

/// `[durable] mode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DurableMode {
    /// In process, no durability.
    #[default]
    None,
    /// The Temporal SDK's ephemeral dev server (development only, Q3).
    Embedded,
    /// A Temporal service, Temporal Cloud included.
    Remote,
}

/// `[durable]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DurableConfig {
    /// The mode.
    pub mode: DurableMode,
    /// The Temporal connection for `remote` (and the task queue for both Temporal modes).
    pub temporal: TemporalConfig,
    /// Activity retries (R22.3).
    pub retry: RetryConfig,
}

/// `[durable.temporal]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TemporalConfig {
    /// The service address, by reference (it may embed an account id).
    pub address: Option<SecretRef>,
    /// The namespace, by reference.
    pub namespace: Option<SecretRef>,
    /// The API key, by reference.
    pub api_key: Option<SecretRef>,
    /// Read `TEMPORAL_ADDRESS`, `TEMPORAL_NAMESPACE` and `TEMPORAL_API_KEY` through the
    /// SDK's own environment loader instead of the references above.
    pub from_env: bool,
    /// The task queue.
    pub task_queue: String,
}

impl Default for TemporalConfig {
    fn default() -> Self {
        Self {
            address: None,
            namespace: None,
            api_key: None,
            from_env: false,
            task_queue: "rusty-harness".into(),
        }
    }
}

/// `[persistence]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PersistenceConfig {
    /// `memory` or `file`.
    pub store: String,
    /// The data directory for the file store.
    pub path: Option<PathBuf>,
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self {
            store: "memory".into(),
            path: None,
        }
    }
}

/// `[plugins]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PluginsConfig {
    /// Directories scanned for Agent Plugins.
    pub paths: Vec<PathBuf>,
}

/// `[agents.<name>]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubAgentConfig {
    /// Where the sub-agent's A2A agent card is served.
    pub card_url: url::Url,
    /// The credential to present, by reference.
    #[serde(default)]
    pub api_key: Option<SecretRef>,
}

/// `[hooks]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HooksConfig {
    /// Hook ids in the order they run when priorities tie (R4.5).
    pub order: Vec<String>,
    /// Failure policy per point, `fail-closed` or `log-and-continue` (R4.6); the
    /// security-relevant points cannot be lowered.
    pub policies: BTreeMap<String, String>,
}

/// `[tools]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ToolsConfig {
    /// A tool result larger than this, in bytes, goes through the oversize policy (R8.6).
    pub max_result_bytes: usize,
    /// `truncate` (keep the head and the tail) or `summarise` (one call to the `default`
    /// model with `prompts/tool/summarise-result.md`).
    pub oversize: OversizePolicyConfig,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            max_result_bytes: 64 * 1024,
            oversize: OversizePolicyConfig::Truncate,
        }
    }
}

/// `[tools] oversize`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OversizePolicyConfig {
    /// Keep the head and the tail, mark the cut.
    #[default]
    Truncate,
    /// Ask a model for a shorter version; truncate when that fails.
    Summarise,
}

/// `[context]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ContextConfig {
    /// The positions in order (R15.1).
    pub order: Vec<String>,
    /// Compaction (R16).
    pub compaction: CompactionConfig,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            order: [
                "system_prompt",
                "system_extensions",
                "participant_roles",
                "skills_index",
                "sub_agent_index",
                "task",
                "plan",
                "state_excerpt",
                "history",
                "tool_results",
            ]
            .map(String::from)
            .to_vec(),
            compaction: CompactionConfig::default(),
        }
    }
}

/// `[context.compaction]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CompactionConfig {
    /// Compact when the estimate exceeds this fraction of the window (R16.1).
    pub threshold: f64,
    /// Recent turns kept verbatim (R16.2).
    pub keep_recent_turns: u32,
    /// The model alias that summarises; the active model when absent.
    pub model: Option<String>,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            threshold: 0.75,
            keep_recent_turns: 8,
            model: None,
        }
    }
}

/// `[loop]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LoopConfig {
    /// Model calls per task before the limit outcome (R11.4).
    pub max_iterations: u32,
    /// Total tokens per task, when set.
    pub max_tokens: Option<u64>,
    /// Wall clock per task, when set.
    pub wall_clock: Option<DurationStr>,
    /// A task that has not progressed for this long is stalled (R23.3).
    pub stall_after: DurationStr,
    /// How long a long-running task waits for the next turn before the idle outcome.
    pub idle_timeout: Option<DurationStr>,
    /// `fail` or `input-required` when a limit is reached.
    pub on_limit: String,
    /// How long a signalled inbox event may wait before the heartbeat re-signals it.
    pub signal_grace: DurationStr,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            max_tokens: None,
            wall_clock: None,
            stall_after: DurationStr::secs(15 * 60),
            idle_timeout: None,
            on_limit: "input-required".into(),
            signal_grace: DurationStr::secs(30),
        }
    }
}

/// `[voice]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct VoiceConfig {
    /// `openai-realtime`, or absent for no voice.
    pub provider: Option<String>,
    /// The model.
    pub model: Option<String>,
    /// The credential, by reference.
    pub api_key: Option<SecretRef>,
}

/// `[o11y]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct O11yConfig {
    /// The OTLP endpoint; no export when absent (R24.2).
    pub otlp_endpoint: Option<url::Url>,
    /// The log level.
    pub log_level: String,
    /// Add Langfuse's generation attributes to the spans (R24.3).
    pub langfuse: bool,
}

impl Default for O11yConfig {
    fn default() -> Self {
        Self {
            otlp_endpoint: None,
            log_level: "info".into(),
            langfuse: false,
        }
    }
}

/// `[tui]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TuiConfig {
    /// The push-to-talk key (R27.5).
    pub push_to_talk_key: String,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            push_to_talk_key: "space".into(),
        }
    }
}

/// Where a configuration comes from.
#[derive(Clone, Debug, Default)]
pub struct ConfigSources {
    /// The TOML file, when one is used.
    pub file: Option<PathBuf>,
    /// Read `RH__…` variables from the process environment.
    pub env: bool,
    /// Code overrides: whatever this holds wins over the file and the environment.
    pub overrides: Option<toml::Table>,
}

/// A configuration that could not be loaded. Never carries a configuration value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// The file could not be read.
    #[error("config file {path}: {reason}")]
    Read {
        /// The file.
        path: PathBuf,
        /// Why.
        reason: String,
    },
    /// The TOML does not parse.
    #[error("config file does not parse: {0}")]
    Parse(String),
    /// A value is of the wrong shape, a field is unknown, or a credential is a literal.
    #[error("config: {0}")]
    Invalid(String),
}

/// The environment prefix: `RH__SECTION__KEY=value`.
pub const ENV_PREFIX: &str = "RH__";

fn env_table(vars: impl IntoIterator<Item = (String, String)>) -> toml::Table {
    let mut root = toml::Table::new();
    for (key, value) in vars {
        let Some(path) = key.strip_prefix(ENV_PREFIX) else {
            continue;
        };
        let segments: Vec<String> = path.split("__").map(str::to_ascii_lowercase).collect();
        if segments.iter().any(String::is_empty) {
            continue;
        }
        let parsed = value
            .parse::<toml::Value>()
            .unwrap_or(toml::Value::String(value.clone()));
        let parsed = match parsed {
            toml::Value::Table(_) | toml::Value::Datetime(_) => toml::Value::String(value),
            other => other,
        };
        let mut table = &mut root;
        for segment in &segments[..segments.len() - 1] {
            let entry = table
                .entry(segment.clone())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            if !entry.is_table() {
                *entry = toml::Value::Table(toml::Table::new());
            }
            table = entry.as_table_mut().expect("just made a table");
        }
        table.insert(segments[segments.len() - 1].clone(), parsed);
    }
    root
}

fn merge(base: &mut toml::Table, overlay: toml::Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

impl Config {
    /// Builds a configuration from a TOML text, environment variables and code
    /// overrides, in that precedence (R30.4). Pure: the file is passed as text and the
    /// environment as pairs, so tests inject both.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] when the text is not TOML; [`ConfigError::Invalid`] for an
    /// unknown field, a wrong type, or a literal where a credential reference belongs.
    pub fn from_layers(
        file: Option<&str>,
        env: impl IntoIterator<Item = (String, String)>,
        overrides: Option<toml::Table>,
    ) -> Result<Self, ConfigError> {
        let mut table = toml::Table::new();
        if let Some(text) = file {
            let parsed: toml::Table =
                toml::from_str(text).map_err(|e| ConfigError::Parse(e.message().to_owned()))?;
            merge(&mut table, parsed);
        }
        merge(&mut table, env_table(env));
        if let Some(overrides) = overrides {
            merge(&mut table, overrides);
        }
        table
            .try_into()
            .map_err(|e: toml::de::Error| ConfigError::Invalid(e.message().to_owned()))
    }

    /// Loads from [`ConfigSources`] through the host's file system (R27.2).
    ///
    /// # Errors
    ///
    /// [`ConfigError::Read`] when the named file cannot be read; otherwise as
    /// [`Config::from_layers`].
    pub async fn load(sources: &ConfigSources, fs: &dyn FileSystem) -> Result<Self, ConfigError> {
        let text = match &sources.file {
            Some(path) => Some(
                String::from_utf8(
                    fs.read(path)
                        .await
                        .map_err(|e| read_error(path, &e))?
                        .to_vec(),
                )
                .map_err(|_| ConfigError::Read {
                    path: path.clone(),
                    reason: "not UTF-8".into(),
                })?,
            ),
            None => None,
        };
        let env: Vec<(String, String)> = if sources.env {
            std::env::vars().collect()
        } else {
            Vec::new()
        };
        Self::from_layers(text.as_deref(), env, sources.overrides.clone())
    }

    /// Writes the commented defaults to `path` (R27.2).
    ///
    /// # Errors
    ///
    /// [`ConfigError::Read`] (as a write failure) when the file cannot be written.
    pub async fn write_default(path: &Path, fs: &dyn FileSystem) -> Result<(), ConfigError> {
        fs.write_atomic(
            path,
            bytes::Bytes::from_static(DEFAULT_CONFIG_TOML.as_bytes()),
        )
        .await
        .map_err(|e| read_error(path, &e))
    }

    /// The JSON schema of the whole configuration (Q9), as published in
    /// `rusty-harness.schema.json`.
    #[must_use]
    pub fn schema() -> schemars::Schema {
        schemars::schema_for!(Config)
    }
}

fn read_error(path: &Path, err: &HostError) -> ConfigError {
    ConfigError::Read {
        path: path.to_path_buf(),
        reason: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigError, DEFAULT_CONFIG_TOML, DurableMode, ModelProvider};
    use crate::host::SecretRef;
    use std::path::Path;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    const FILE: &str = r#"
[agent]
name = "demo"
[models.default]
provider = "openai"
model = "gpt-6.1"
api_key = "env:OPENAI_API_KEY"
[loop]
max_iterations = 10
"#;

    #[test]
    fn precedence_is_defaults_then_file_then_env_then_code() {
        let mut overrides = toml::Table::new();
        let mut lp = toml::Table::new();
        lp.insert("max_iterations".into(), toml::Value::Integer(99));
        overrides.insert("loop".into(), toml::Value::Table(lp));
        let config = Config::from_layers(
            Some(FILE),
            env(&[
                ("RH__LOOP__MAX_ITERATIONS", "20"),
                ("RH__MODELS__DEFAULT__MODEL", "gpt-6.1-mini"),
                ("RH__DURABLE__MODE", "embedded"),
            ]),
            Some(overrides),
        )
        .expect("loads");
        assert_eq!(config.agent.name, "demo");
        assert_eq!(
            config.r#loop.max_iterations, 99,
            "code beats env beats file"
        );
        assert_eq!(
            config.models["default"].model, "gpt-6.1-mini",
            "env beats file"
        );
        assert_eq!(config.models["default"].provider, ModelProvider::Openai);
        assert_eq!(
            config.models["default"].api_key,
            SecretRef::Env("OPENAI_API_KEY".into())
        );
        assert_eq!(config.durable.mode, DurableMode::Embedded);
        assert_eq!(
            config.server.bind, "127.0.0.1:7474",
            "defaults fill the rest"
        );
        assert_eq!(
            Config::from_layers(None, Vec::new(), None).expect("empty"),
            Config::default()
        );
    }

    #[test]
    fn environment_values_are_typed_and_unrelated_variables_are_ignored() {
        let config = Config::from_layers(
            None,
            env(&[
                ("RH__O11Y__LANGFUSE", "true"),
                ("RH__SERVER__BINDINGS", "[\"rest\"]"),
                ("RH__LOCALE", "fr"),
                ("HOME", "/nope"),
                ("RH__", "x"),
            ]),
            None,
        )
        .expect("loads");
        assert!(config.o11y.langfuse);
        assert_eq!(config.server.bindings, ["rest"]);
        assert_eq!(config.locale.as_deref(), Some("fr"));
    }

    #[test]
    fn unknown_fields_and_wrong_types_are_refused() {
        let err = Config::from_layers(Some("[agent]\nnmae = \"x\"\n"), Vec::new(), None)
            .expect_err("unknown field");
        assert!(
            matches!(err, ConfigError::Invalid(ref m) if m.contains("nmae")),
            "{err}"
        );
        let err = Config::from_layers(
            Some("[loop]\nmax_iterations = \"many\"\n"),
            Vec::new(),
            None,
        )
        .expect_err("wrong type");
        assert!(matches!(err, ConfigError::Invalid(_)));
        assert!(matches!(
            Config::from_layers(Some("not = = toml"), Vec::new(), None),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn abuse_literal_credential_in_config_is_refused() {
        // Abuse case 7 / fail-closed: a literal key where a reference belongs never loads,
        // and the error never echoes it.
        let text = "[models.default]\nprovider = \"openai\"\nmodel = \"m\"\napi_key = \"sk-live-canary-123\"\n";
        let err = Config::from_layers(Some(text), Vec::new(), None).expect_err("literal refused");
        assert!(matches!(err, ConfigError::Invalid(_)));
        assert!(!err.to_string().contains("canary"), "{err}");
        let err = Config::from_layers(
            None,
            env(&[("RH__VOICE__API_KEY", "sk-live-canary-456")]),
            None,
        )
        .expect_err("literal from env refused");
        assert!(!err.to_string().contains("canary"), "{err}");
    }

    #[test]
    fn snapshot_default_config_toml() {
        let parsed = Config::from_layers(Some(DEFAULT_CONFIG_TOML), Vec::new(), None)
            .expect("the template parses");
        assert_eq!(
            parsed,
            Config::default(),
            "config.default.toml must express exactly Config::default()"
        );
    }

    #[test]
    fn snapshot_config_schema() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("rusty-harness.schema.json");
        let mut json = serde_json::to_string_pretty(&Config::schema()).expect("serializes");
        json.push('\n');
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            std::fs::write(&path, &json).expect("writes the schema");
        }
        let published: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default())
                .expect("rusty-harness.schema.json is JSON");
        let generated: serde_json::Value =
            serde_json::from_str(&json).expect("the generated schema is JSON");
        assert_eq!(
            published, generated,
            "rusty-harness.schema.json is stale: rerun with UPDATE_SNAPSHOTS=1"
        );
        assert!(
            json.contains("\"secret-ref\""),
            "SecretRef fields carry format: secret-ref"
        );
    }
}

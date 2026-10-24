//! Agent Plugins: discovery, validation and all-or-nothing installation (R5).
//!
//! A plugin is a directory with `plugin.json` (Agent Plugins 1.0.0), skills under
//! `skills/<name>/SKILL.md`, MCP servers in `mcp.json`, and rusty-harness's own content
//! under the `dev.rusty-harness` namespace (Q17): remote hooks, commands and prompt
//! fragments. [`PluginLoader::validate`] is a pure function run before any registration;
//! [`PluginLoader::install`] registers into the registry only after every entity of the
//! plugin was checked for conflicts, so a failing plugin leaves nothing behind (R5.4).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::assets::{PromptFileError, PromptFragment, PromptRegistry};
use crate::harness::hooks::{HookPoint, Phase};
use crate::harness::skills::{Skill, SkillError, SkillMeta, SkillParser, SkillRoot};
use crate::host::FileSystem;
use crate::registry::{Entity, EntityId, EntityKind, Provenance, Registry, RegistryError};

/// The manifest schema id the specification requires.
pub const SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";

/// rusty-harness's client namespace inside a plugin (Q17).
pub const NAMESPACE: &str = "dev.rusty-harness";

/// `plugin.json`. Unknown top-level fields are reported and ignored, as the specification
/// says; everything else is checked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Must equal [`SCHEMA`].
    #[serde(rename = "$schema")]
    pub schema: String,
    /// 1–64 chars of lowercase alphanumerics, hyphens and periods.
    pub name: String,
    /// Semantic version, recommended.
    #[serde(default)]
    pub version: Option<String>,
    /// Purpose.
    #[serde(default)]
    pub description: Option<String>,
    /// Author.
    #[serde(default)]
    pub author: Option<Author>,
    /// Homepage.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Source repository.
    #[serde(default)]
    pub repository: Option<String>,
    /// SPDX identifier, recommended.
    #[serde(default)]
    pub license: Option<String>,
    /// Discovery tags.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Client-specific data by reverse-domain namespace.
    #[serde(default)]
    pub extensions: BTreeMap<String, serde_json::Value>,
    /// Fields the specification does not define: reported, then ignored.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, serde_json::Value>,
}

/// `author` in the manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Author {
    /// Name.
    #[serde(default)]
    pub name: Option<String>,
    /// Email.
    #[serde(default)]
    pub email: Option<String>,
    /// URL.
    #[serde(default)]
    pub url: Option<String>,
}

/// `mcp.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct McpConfig {
    /// The schema id, when present.
    #[serde(rename = "$schema", default)]
    pub schema: Option<String>,
    /// Servers by name.
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: BTreeMap<String, McpServerSpec>,
}

/// One MCP server of a plugin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum McpServerSpec {
    /// A child process speaking MCP over stdio.
    Stdio {
        /// The executable: a bare name on `PATH` or a path under the plugin root.
        command: String,
        /// Arguments (`${PLUGIN_ROOT}`/`${PLUGIN_DATA}` substituted).
        #[serde(default)]
        args: Vec<String>,
        /// Environment (values substituted; keys may not name the two variables).
        #[serde(default)]
        env: BTreeMap<String, String>,
        /// Working directory (substituted).
        #[serde(default)]
        cwd: Option<String>,
    },
    /// MCP's streamable HTTP transport.
    StreamableHttp {
        /// The endpoint: HTTPS, or HTTP to `localhost` / a loopback address only.
        url: String,
        /// Extra headers.
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    /// The legacy HTTP+SSE transport, which this harness does not support.
    Sse {
        /// The endpoint.
        url: String,
    },
}

/// A remote hook a plugin declares (`dev.rusty-harness/hooks.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteHookSpec {
    /// The hook's id within the plugin.
    pub id: String,
    /// The catalogue point, dotted (`tool.execute`).
    pub point: String,
    /// `pre`, `guard`, `in` or `post`.
    pub phase: Phase,
    /// How to reach it.
    pub transport: RemoteHookTransport,
    /// Priority (lower runs first).
    #[serde(default)]
    pub priority: i32,
    /// Hooks that must run before this one.
    #[serde(default)]
    pub after: Vec<String>,
}

/// Where a declared hook runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum RemoteHookTransport {
    /// A JSON-RPC 2.0 endpoint.
    JsonRpc {
        /// The URL (HTTPS, or HTTP to loopback).
        url: String,
    },
    /// A tool on one of the plugin's MCP servers.
    Mcp {
        /// The server name in `mcp.json`.
        server: String,
    },
}

/// A command a plugin declares (`dev.rusty-harness/commands.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    /// The name after the slash.
    pub name: String,
    /// What it does, for the palette.
    pub description: String,
    /// Roles that may invoke it; empty means every participant.
    #[serde(default)]
    pub roles: Vec<String>,
    /// How it runs.
    pub handler: RemoteHookTransport,
}

/// rusty-harness's namespace inside a plugin.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RustyHarnessExtensions {
    /// Declared remote hooks.
    pub hooks: Vec<RemoteHookSpec>,
    /// Declared commands.
    pub commands: Vec<CommandSpec>,
    /// Prompt fragments: crate-relative path under `dev.rusty-harness/prompts/` and text.
    pub prompts: Vec<(String, String)>,
}

/// Where a plugin's files live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginRoot {
    /// A directory on the host's file system.
    Dir(PathBuf),
    /// Supplied programmatically (R5.3).
    InMemory,
}

/// A plugin, discovered or supplied, before validation.
#[derive(Clone, Debug)]
pub struct PluginSpec {
    /// The manifest.
    pub manifest: PluginManifest,
    /// Where it lives.
    pub root: PluginRoot,
    /// Skills: directory name, `SKILL.md` text, files for in-memory roots.
    pub skills: Vec<SkillSpec>,
    /// MCP servers.
    pub mcp_servers: BTreeMap<String, McpServerSpec>,
    /// The `dev.rusty-harness` content.
    pub extensions: RustyHarnessExtensions,
}

/// One skill of a plugin, as discovered.
#[derive(Clone, Debug)]
pub struct SkillSpec {
    /// The directory name under `skills/`.
    pub dir_name: String,
    /// The `SKILL.md` text.
    pub skill_md: String,
    /// The files, for an in-memory plugin.
    pub files: BTreeMap<PathBuf, Bytes>,
}

/// A plugin that is refused, whole (R5.4). Names the plugin and the field.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PluginError {
    /// The manifest or a declared file is invalid.
    #[error("plugin {plugin}: {field}: {reason}")]
    Invalid {
        /// The plugin name, or its directory when the name is unreadable.
        plugin: String,
        /// The field or file.
        field: String,
        /// What is wrong.
        reason: String,
    },
    /// A skill of the plugin is invalid.
    #[error("plugin {plugin}: {source}")]
    Skill {
        /// The plugin.
        plugin: String,
        /// The skill's error.
        source: Box<SkillError>,
    },
    /// A prompt fragment of the plugin is invalid.
    #[error("plugin {plugin}: {source}")]
    Prompt {
        /// The plugin.
        plugin: String,
        /// The fragment's error.
        source: Box<PromptFileError>,
    },
    /// An entity of the plugin collides with one already registered (R5.6).
    #[error("plugin {plugin}: {source}")]
    Conflict {
        /// The plugin.
        plugin: String,
        /// The registry's error, naming the holder.
        source: Box<RegistryError>,
    },
}

fn invalid(plugin: &str, field: &str, reason: impl Into<String>) -> PluginError {
    PluginError::Invalid {
        plugin: plugin.to_owned(),
        field: field.to_owned(),
        reason: reason.into(),
    }
}

/// An MCP server a plugin contributed, registered for the tools module to connect to
/// (task 15). The spec is kept with `${PLUGIN_ROOT}`/`${PLUGIN_DATA}` already substituted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServerEntity {
    id: EntityId,
    /// The server, substituted.
    pub spec: McpServerSpec,
    /// The plugin's root, for relative commands.
    pub root: PluginRoot,
}

impl Entity for McpServerEntity {
    fn kind(&self) -> EntityKind {
        EntityKind::Tool
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

/// A remote hook a plugin declared, registered for the hooks module to install once the
/// operation types exist (`hooks::remote::install_declared`).
#[derive(Clone, Debug, PartialEq)]
pub struct DeclaredHook {
    id: EntityId,
    /// The declaration.
    pub spec: RemoteHookSpec,
    /// The resolved point.
    pub point: HookPoint,
    /// The plugin that declared it.
    pub plugin: String,
}

impl Entity for DeclaredHook {
    fn kind(&self) -> EntityKind {
        EntityKind::Hook
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

/// A command a plugin declared, registered for the commands module (task 13).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredCommand {
    id: EntityId,
    /// The declaration.
    pub spec: CommandSpec,
    /// The plugin that declared it.
    pub plugin: String,
}

impl Entity for DeclaredCommand {
    fn kind(&self) -> EntityKind {
        EntityKind::Command
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

/// What `install` registered, for the record and for uninstalling later.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstalledPlugin {
    /// The plugin's name.
    pub name: String,
    /// Skills registered.
    pub skills: Vec<EntityId>,
    /// MCP servers registered.
    pub mcp_servers: Vec<EntityId>,
    /// Hooks declared.
    pub hooks: Vec<EntityId>,
    /// Commands declared.
    pub commands: Vec<EntityId>,
    /// Prompt fragments added (they replace built-ins with the same id and locale).
    pub prompts: Vec<EntityId>,
}

/// Discovers, validates and installs plugins.
#[derive(Debug, Clone)]
pub struct PluginLoader {
    /// Where `PLUGIN_DATA` directories live: `<data_dir>/plugins/<name>/`.
    pub data_dir: PathBuf,
}

impl PluginLoader {
    /// A loader whose `PLUGIN_DATA` directories live under `data_dir`.
    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    /// Reads every plugin directory (a child with a `plugin.json`) under `paths` (R5.1).
    ///
    /// # Errors
    ///
    /// [`PluginError`] for a directory whose files cannot be read or parsed; other
    /// plugins are still returned, so one bad plugin does not hide the rest.
    pub async fn discover(
        &self,
        paths: &[PathBuf],
        fs: &dyn FileSystem,
    ) -> (Vec<PluginSpec>, Vec<PluginError>) {
        let mut specs = Vec::new();
        let mut errors = Vec::new();
        for path in paths {
            let Ok(children) = fs.list(path).await else {
                continue;
            };
            for child in children {
                if fs.read(&child.join("plugin.json")).await.is_err() {
                    continue;
                }
                match Self::read_plugin(&child, fs).await {
                    Ok(spec) => specs.push(spec),
                    Err(err) => errors.push(err),
                }
            }
        }
        (specs, errors)
    }

    async fn read_plugin(dir: &Path, fs: &dyn FileSystem) -> Result<PluginSpec, PluginError> {
        let dir_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let read_text = |rel: &str| {
            let path = dir.join(rel);
            async move {
                fs.read(&path)
                    .await
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
            }
        };
        let manifest_text = read_text("plugin.json")
            .await
            .map_err(|e| invalid(&dir_name, "plugin.json", e.to_string()))?;
        let manifest: PluginManifest = serde_json::from_str(&manifest_text)
            .map_err(|e| invalid(&dir_name, "plugin.json", e.to_string()))?;
        let name = manifest.name.clone();
        for unknown in manifest.unknown.keys() {
            tracing::warn!(plugin = %name, field = %unknown, "plugin.json: unknown field ignored");
        }
        let mcp_servers = match read_text("mcp.json").await {
            Ok(text) => {
                serde_json::from_str::<McpConfig>(&text)
                    .map_err(|e| invalid(&name, "mcp.json", e.to_string()))?
                    .mcp_servers
            }
            Err(_) => BTreeMap::new(),
        };
        let mut skills = Vec::new();
        if let Ok(entries) = fs.list(&dir.join("skills")).await {
            for entry in entries {
                let skill_dir_name = entry
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Ok(skill_md) = fs.read(&entry.join("SKILL.md")).await {
                    skills.push(SkillSpec {
                        dir_name: skill_dir_name,
                        skill_md: String::from_utf8_lossy(&skill_md).into_owned(),
                        files: BTreeMap::new(),
                    });
                }
            }
        }
        let ns = dir.join(NAMESPACE);
        let mut extensions = RustyHarnessExtensions::default();
        if let Ok(text) = read_text(&format!("{NAMESPACE}/hooks.json")).await {
            extensions.hooks = serde_json::from_str(&text)
                .map_err(|e| invalid(&name, "dev.rusty-harness/hooks.json", e.to_string()))?;
        }
        if let Ok(text) = read_text(&format!("{NAMESPACE}/commands.json")).await {
            extensions.commands = serde_json::from_str(&text)
                .map_err(|e| invalid(&name, "dev.rusty-harness/commands.json", e.to_string()))?;
        }
        let prompts_dir = ns.join("prompts");
        let mut pending = vec![prompts_dir.clone()];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = fs.list(&dir).await else {
                continue;
            };
            for entry in entries {
                if entry.extension().is_some_and(|e| e == "md") {
                    if let Ok(text) = fs.read(&entry).await {
                        let rel = entry
                            .strip_prefix(&prompts_dir)
                            .unwrap_or(&entry)
                            .to_string_lossy()
                            .replace('\\', "/");
                        extensions
                            .prompts
                            .push((rel, String::from_utf8_lossy(&text).into_owned()));
                    }
                } else {
                    pending.push(entry);
                }
            }
        }
        extensions.prompts.sort();
        Ok(PluginSpec {
            manifest,
            root: PluginRoot::Dir(dir.to_path_buf()),
            skills,
            mcp_servers,
            extensions,
        })
    }

    /// Checks everything the specification and the security design require, touching
    /// nothing (R5.4, abuse 6).
    ///
    /// # Errors
    ///
    /// [`PluginError`] naming the first offending field.
    pub fn validate(spec: &PluginSpec) -> Result<(), PluginError> {
        let m = &spec.manifest;
        let name = &m.name;
        if m.schema != SCHEMA {
            return Err(invalid(name, "$schema", format!("must be {SCHEMA}")));
        }
        validate_plugin_name(name)?;
        if let Some(version) = &m.version
            && version.parse::<semver::Version>().is_err()
        {
            return Err(invalid(name, "version", "not a semantic version"));
        }
        for (server, server_spec) in &spec.mcp_servers {
            let field = format!("mcp.json/{server}");
            match server_spec {
                McpServerSpec::Stdio {
                    command, env, cwd, ..
                } => {
                    if command.contains('/') || command.contains('\\') {
                        inside_root(name, &field, command)?;
                    }
                    if let Some(cwd) = cwd {
                        inside_root(name, &field, cwd)?;
                    }
                    if env.keys().any(|k| k == "PLUGIN_ROOT" || k == "PLUGIN_DATA") {
                        return Err(invalid(
                            name,
                            &field,
                            "env must not name PLUGIN_ROOT or PLUGIN_DATA",
                        ));
                    }
                }
                McpServerSpec::StreamableHttp { url, .. } => https_or_loopback(name, &field, url)?,
                McpServerSpec::Sse { .. } => {
                    return Err(invalid(
                        name,
                        &field,
                        "the sse transport is not supported; use streamable-http",
                    ));
                }
            }
        }
        for hook in &spec.extensions.hooks {
            let field = format!("hooks/{}", hook.id);
            if HookPoint::from_name(&hook.point).is_none() {
                return Err(invalid(
                    name,
                    &field,
                    format!("{:?} is not a catalogue point", hook.point),
                ));
            }
            validate_transport(name, &field, &hook.transport, &spec.mcp_servers)?;
        }
        for command in &spec.extensions.commands {
            let field = format!("commands/{}", command.name);
            if command.name.is_empty()
                || !command
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(invalid(
                    name,
                    &field,
                    "command names are alphanumeric with hyphens and underscores",
                ));
            }
            validate_transport(name, &field, &command.handler, &spec.mcp_servers)?;
        }
        for skill in &spec.skills {
            SkillParser::parse(&skill.dir_name, &skill.skill_md).map_err(|source| {
                PluginError::Skill {
                    plugin: name.clone(),
                    source: Box::new(source),
                }
            })?;
        }
        for (rel, text) in &spec.extensions.prompts {
            PromptFragment::parse(rel, text).map_err(|source| PluginError::Prompt {
                plugin: name.clone(),
                source: Box::new(source),
            })?;
        }
        Ok(())
    }

    /// Validates, checks every identity for conflicts, then registers every entity of the
    /// plugin with the plugin's version as its registry version (R5.3, R5.5, R5.6).
    ///
    /// # Errors
    ///
    /// [`PluginError`]: nothing was registered.
    pub fn install(
        &self,
        spec: &PluginSpec,
        registry: &Registry,
        prompts: &mut PromptRegistry,
    ) -> Result<InstalledPlugin, PluginError> {
        Self::validate(spec)?;
        let staged = self.stage(spec)?;
        staged.ensure_free(registry)?;
        staged.register(registry, prompts)
    }

    /// Builds every entity of the plugin without registering any.
    fn stage(&self, spec: &PluginSpec) -> Result<Staged, PluginError> {
        let name = spec.manifest.name.clone();
        let version = spec
            .manifest
            .version
            .as_deref()
            .and_then(|v| v.parse::<semver::Version>().ok());
        let id = |s: &str| match &version {
            Some(v) => EntityId::versioned(s, v.clone()),
            None => EntityId::new(s),
        };
        let plugin_root = match &spec.root {
            PluginRoot::Dir(dir) => dir.to_string_lossy().into_owned(),
            PluginRoot::InMemory => format!("<memory:{name}>"),
        };
        let plugin_data = self
            .data_dir
            .join("plugins")
            .join(&name)
            .to_string_lossy()
            .into_owned();
        let substitute = |s: &str| {
            s.replace("${PLUGIN_ROOT}", &plugin_root)
                .replace("${PLUGIN_DATA}", &plugin_data)
        };

        let skills = stage_skills(spec, &name, version.as_ref())?;
        let servers = stage_servers(spec, &name, &id, &substitute);
        let mut hooks = Vec::new();
        for h in &spec.extensions.hooks {
            let point = HookPoint::from_name(&h.point).ok_or_else(|| {
                invalid(&name, &format!("hooks/{}", h.id), "not a catalogue point")
            })?;
            hooks.push(Arc::new(DeclaredHook {
                id: id(&format!("{name}/{}", h.id)),
                spec: h.clone(),
                point,
                plugin: name.clone(),
            }));
        }
        let commands: Vec<Arc<DeclaredCommand>> = spec
            .extensions
            .commands
            .iter()
            .map(|c| {
                Arc::new(DeclaredCommand {
                    id: id(&c.name),
                    spec: c.clone(),
                    plugin: name.clone(),
                })
            })
            .collect();
        let mut fragments = Vec::new();
        for (rel, text) in &spec.extensions.prompts {
            fragments.push(PromptFragment::parse(rel, text).map_err(|source| {
                PluginError::Prompt {
                    plugin: name.clone(),
                    source: Box::new(source),
                }
            })?);
        }
        Ok(Staged {
            provenance: Provenance::Plugin(id(&name)),
            name,
            skills,
            servers,
            hooks,
            commands,
            fragments,
        })
    }
}

fn stage_skills(
    spec: &PluginSpec,
    name: &str,
    version: Option<&semver::Version>,
) -> Result<Vec<Arc<Skill>>, PluginError> {
    let mut skills = Vec::new();
    for skill in &spec.skills {
        let meta: SkillMeta =
            SkillParser::parse(&skill.dir_name, &skill.skill_md).map_err(|source| {
                PluginError::Skill {
                    plugin: name.to_owned(),
                    source: Box::new(source),
                }
            })?;
        let root = match &spec.root {
            PluginRoot::Dir(dir) => SkillRoot::Dir(dir.join("skills").join(&skill.dir_name)),
            PluginRoot::InMemory => {
                let mut files = skill.files.clone();
                files
                    .entry(PathBuf::from("SKILL.md"))
                    .or_insert_with(|| Bytes::from(skill.skill_md.clone()));
                SkillRoot::InMemory(files)
            }
        };
        skills.push(Arc::new(Skill::new(meta, root, version.cloned())));
    }
    Ok(skills)
}

fn stage_servers(
    spec: &PluginSpec,
    name: &str,
    id: &dyn Fn(&str) -> EntityId,
    substitute: &dyn Fn(&str) -> String,
) -> Vec<Arc<McpServerEntity>> {
    let mut servers = Vec::new();
    for (server_name, server_spec) in &spec.mcp_servers {
        let substituted = match server_spec.clone() {
            McpServerSpec::Stdio {
                command,
                args,
                env,
                cwd,
            } => McpServerSpec::Stdio {
                command: substitute(&command),
                args: args.iter().map(|a| substitute(a)).collect(),
                env: env
                    .iter()
                    .map(|(k, v)| (k.clone(), substitute(v)))
                    .collect(),
                cwd: cwd.as_deref().map(substitute),
            },
            other => other,
        };
        servers.push(Arc::new(McpServerEntity {
            id: id(&format!("{name}/{server_name}")),
            spec: substituted,
            root: spec.root.clone(),
        }));
    }
    servers
}

/// Every entity of a plugin, built but not yet registered.
struct Staged {
    name: String,
    provenance: Provenance,
    skills: Vec<Arc<Skill>>,
    servers: Vec<Arc<McpServerEntity>>,
    hooks: Vec<Arc<DeclaredHook>>,
    commands: Vec<Arc<DeclaredCommand>>,
    fragments: Vec<PromptFragment>,
}

impl Staged {
    fn conflict(&self, source: RegistryError) -> PluginError {
        PluginError::Conflict {
            plugin: self.name.clone(),
            source: Box::new(source),
        }
    }

    /// Checks every identity before registering any (all or nothing).
    fn ensure_free(&self, registry: &Registry) -> Result<(), PluginError> {
        for skill in &self.skills {
            registry
                .ensure_free::<Skill>(skill.entity_id(), EntityKind::Skill)
                .map_err(|e| self.conflict(e))?;
        }
        for server in &self.servers {
            registry
                .ensure_free::<McpServerEntity>(server.entity_id(), EntityKind::Tool)
                .map_err(|e| self.conflict(e))?;
        }
        for hook in &self.hooks {
            registry
                .ensure_free::<DeclaredHook>(hook.entity_id(), EntityKind::Hook)
                .map_err(|e| self.conflict(e))?;
        }
        for command in &self.commands {
            registry
                .ensure_free::<DeclaredCommand>(command.entity_id(), EntityKind::Command)
                .map_err(|e| self.conflict(e))?;
        }
        Ok(())
    }

    fn register(
        self,
        registry: &Registry,
        prompts: &mut PromptRegistry,
    ) -> Result<InstalledPlugin, PluginError> {
        let mut installed = InstalledPlugin {
            name: self.name.clone(),
            ..InstalledPlugin::default()
        };
        for skill in &self.skills {
            installed.skills.push(skill.entity_id().clone());
            registry
                .register_with(Arc::clone(skill), self.provenance.clone())
                .map_err(|e| self.conflict(e))?;
        }
        for server in &self.servers {
            installed.mcp_servers.push(server.entity_id().clone());
            registry
                .register_with(Arc::clone(server), self.provenance.clone())
                .map_err(|e| self.conflict(e))?;
        }
        for hook in &self.hooks {
            installed.hooks.push(hook.entity_id().clone());
            registry
                .register_with(Arc::clone(hook), self.provenance.clone())
                .map_err(|e| self.conflict(e))?;
        }
        for command in &self.commands {
            installed.commands.push(command.entity_id().clone());
            registry
                .register_with(Arc::clone(command), self.provenance.clone())
                .map_err(|e| self.conflict(e))?;
        }
        for fragment in self.fragments {
            installed.prompts.push(fragment.entity_id().clone());
            prompts.add(fragment);
        }
        Ok(installed)
    }
}

fn validate_plugin_name(name: &str) -> Result<(), PluginError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && !name.contains("--")
        && !name.contains("..");
    if ok {
        Ok(())
    } else {
        Err(invalid(
            name,
            "name",
            "must be 1–64 lowercase alphanumerics, hyphens and periods, starting and ending alphanumeric",
        ))
    }
}

fn inside_root(plugin: &str, field: &str, path: &str) -> Result<(), PluginError> {
    let expanded = path
        .replace("${PLUGIN_ROOT}", "ROOT")
        .replace("${PLUGIN_DATA}", "DATA");
    let p = Path::new(&expanded);
    if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(invalid(
            plugin,
            field,
            format!("{path:?} is outside the plugin directory"),
        ));
    }
    Ok(())
}

fn https_or_loopback(plugin: &str, field: &str, url: &str) -> Result<(), PluginError> {
    let parsed: url::Url = url
        .parse()
        .map_err(|_| invalid(plugin, field, format!("{url:?} is not a URL")))?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" => {
            let host = parsed.host_str().unwrap_or("");
            let loopback = host == "localhost"
                || parsed.host().is_some_and(|h| match h {
                    url::Host::Ipv4(ip) => ip.is_loopback(),
                    url::Host::Ipv6(ip) => ip.is_loopback(),
                    url::Host::Domain(_) => false,
                });
            if loopback {
                Ok(())
            } else {
                Err(invalid(
                    plugin,
                    field,
                    format!(
                        "{url:?}: plain http is allowed to localhost or a loopback address only"
                    ),
                ))
            }
        }
        other => Err(invalid(
            plugin,
            field,
            format!("scheme {other:?} is not allowed"),
        )),
    }
}

fn validate_transport(
    plugin: &str,
    field: &str,
    transport: &RemoteHookTransport,
    servers: &BTreeMap<String, McpServerSpec>,
) -> Result<(), PluginError> {
    match transport {
        RemoteHookTransport::JsonRpc { url } => https_or_loopback(plugin, field, url),
        RemoteHookTransport::Mcp { server } => {
            if servers.contains_key(server) {
                Ok(())
            } else {
                Err(invalid(
                    plugin,
                    field,
                    format!("mcp server {server:?} is not in mcp.json"),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PluginLoader, PluginManifest, PluginRoot, PluginSpec, RustyHarnessExtensions, SCHEMA,
        validate_plugin_name,
    };
    use std::collections::BTreeMap;

    fn manifest(name: &str) -> PluginManifest {
        serde_json::from_value(serde_json::json!({ "$schema": SCHEMA, "name": name, "version": "1.2.0", "unknownField": 1 })).unwrap()
    }

    #[test]
    fn the_manifest_parses_keeps_unknown_fields_aside_and_validates_the_name() {
        let m = manifest("demo-plugin");
        assert_eq!(m.unknown.keys().collect::<Vec<_>>(), ["unknownField"]);
        for bad in ["Demo", "-demo", "demo-", "de--mo", "de..mo", ""] {
            assert!(validate_plugin_name(bad).is_err(), "{bad:?}");
        }
        assert!(validate_plugin_name("com.example.demo-1").is_ok());
        let spec = PluginSpec {
            manifest: m,
            root: PluginRoot::InMemory,
            skills: vec![],
            mcp_servers: BTreeMap::new(),
            extensions: RustyHarnessExtensions::default(),
        };
        assert!(PluginLoader::validate(&spec).is_ok());
        let mut wrong_schema = spec;
        wrong_schema.manifest.schema = "https://example.com/other".into();
        assert!(
            PluginLoader::validate(&wrong_schema)
                .unwrap_err()
                .to_string()
                .contains("$schema")
        );
    }
}

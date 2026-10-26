//! Feature: Agent plugins and skills
//! Requirement: docs/specs/issue-3/requirements.md#R5
//!
//! The scenarios of tasks 12 of `tasks.md`, against the fixture plugin under
//! `tests/fixtures/plugins/demo/` read through the host's file system.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusty_harness::assets::{Prompt, PromptArgs, PromptRegistry};
use rusty_harness::harness::plugins::{PluginError, PluginLoader, PluginRoot, PluginSpec};
use rusty_harness::harness::skills::Skill;
use rusty_harness::host::testing::MemFs;
use rusty_harness::host::{FileSystem, Host, TokioFs};
use rusty_harness::registry::{Entity, EntityId, Registry};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins")
}

async fn discover_demo() -> PluginSpec {
    let loader = PluginLoader::new("/tmp/rh-data");
    let (specs, errors) = loader.discover(&[fixtures()], &TokioFs).await;
    assert!(errors.is_empty(), "{errors:?}");
    specs
        .into_iter()
        .find(|s| s.manifest.name == "demo")
        .expect("the demo plugin is discovered")
}

/// Scenario: A plugin with an invalid manifest leaves no entity behind
///     Given the demo plugin with its MCP server's command pointing outside the plugin directory
///     When it is installed
///     Then the whole plugin is refused with an error naming mcp.json and the server
///     And the registry holds none of its skills, servers, hooks or commands
#[tokio::test]
async fn a_plugin_with_an_invalid_manifest_leaves_no_entity_behind() {
    let mut spec = discover_demo().await;
    if let Some(rusty_harness::harness::plugins::McpServerSpec::Stdio { command, .. }) =
        spec.mcp_servers.get_mut("echo")
    {
        *command = "../../../usr/bin/evil".into();
    }
    let registry = Registry::new();
    let mut prompts = PromptRegistry::builtin();
    let err = PluginLoader::new("/tmp/rh-data")
        .install(&spec, &registry, &mut prompts)
        .expect_err("refused");
    assert!(
        matches!(err, PluginError::Invalid { ref field, .. } if field == "mcp.json/echo"),
        "{err}"
    );
    assert!(err.to_string().contains("outside the plugin directory"));
    assert_eq!(registry.list::<Skill>().len(), 0);
    assert_eq!(
        registry
            .list::<rusty_harness::harness::plugins::McpServerEntity>()
            .len(),
        0
    );
    assert_eq!(
        registry
            .list::<rusty_harness::harness::plugins::DeclaredHook>()
            .len(),
        0
    );
    assert_eq!(
        prompts
            .get("system/base", "en")
            .unwrap()
            .entity_id()
            .version
            .as_ref()
            .map(ToString::to_string),
        Some("1.0.0".into()),
        "the built-in prompt is untouched"
    );
}

/// Scenario: A plugin whose manifest escapes its directory is refused whole (abuse case 6)
#[tokio::test]
async fn abuse_plugin_manifest_escaping_its_directory_is_refused_whole() {
    let mut spec = discover_demo().await;
    if let Some(rusty_harness::harness::plugins::McpServerSpec::Stdio { cwd, .. }) =
        spec.mcp_servers.get_mut("echo")
    {
        *cwd = Some("/etc".into());
    }
    assert!(PluginLoader::validate(&spec).is_err());
    let mut spec = discover_demo().await;
    spec.extensions.hooks[0].transport =
        rusty_harness::harness::plugins::RemoteHookTransport::JsonRpc {
            url: "http://hooks.example/audit".into(),
        };
    let err = PluginLoader::validate(&spec).expect_err("plain http off loopback");
    assert!(err.to_string().contains("loopback"), "{err}");
    let mut spec = discover_demo().await;
    spec.extensions.hooks[0].point = "tool.nonexistent".into();
    assert!(
        PluginLoader::validate(&spec)
            .unwrap_err()
            .to_string()
            .contains("not a catalogue point")
    );
}

/// Scenario: A skill body is loaded on activation and not before
///     Given the demo plugin installed from a counting file system
///     When the skill is registered
///     Then only its name and description were read
///     When the skill is activated
///     Then its body is read once, and a resource only on demand
#[tokio::test]
async fn a_skill_body_is_loaded_on_activation_and_not_before() {
    let spec = discover_demo().await;
    let registry = Registry::new();
    let mut prompts = PromptRegistry::builtin();
    let installed = PluginLoader::new("/tmp/rh-data")
        .install(&spec, &registry, &mut prompts)
        .expect("installs");
    assert_eq!(installed.skills.len(), 1);
    let skill = registry
        .get::<Skill>(&EntityId::versioned("greeter", "1.0.0".parse().unwrap()))
        .expect("registered under the plugin's version");
    assert_eq!(
        skill.index_line(),
        "greeter: Greets a participant by name. Use when a greeting is asked for."
    );
    assert!(!skill.body_loaded(), "level 1 only at registration");
    let counting = Arc::new(CountingFs::default());
    let body = skill.body(counting.as_ref()).await.expect("level 2");
    assert!(body.starts_with("# Greeter"));
    assert_eq!(counting.reads(), 1);
    let _ = skill.body(counting.as_ref()).await.expect("cached");
    assert_eq!(counting.reads(), 1, "the body is read once");
    let phrases = skill
        .resource(Path::new("references/PHRASES.md"), counting.as_ref())
        .await
        .expect("level 3");
    assert!(String::from_utf8_lossy(&phrases).contains("welcome"));
    assert!(
        skill
            .resource(Path::new("../plugin.json"), counting.as_ref())
            .await
            .is_err(),
        "resources stay under the skill root"
    );
}

/// Scenario: A registered default is replaced by a plugin without a code change
///     Given the demo plugin carrying its own `system/base` prompt fragment
///     When it is installed
///     Then the harness's default system prompt is the plugin's, with the plugin's version
///     And the MCP server, the hook and the command are registered under the plugin's version
#[tokio::test]
async fn a_registered_default_is_replaced_by_a_plugin_without_a_code_change() {
    let spec = discover_demo().await;
    let registry = Registry::new();
    let mut prompts = PromptRegistry::builtin();
    let installed = PluginLoader::new("/tmp/rh-data")
        .install(&spec, &registry, &mut prompts)
        .expect("installs");
    let base = prompts.get("system/base", "en").expect("present");
    assert_eq!(
        base.entity_id().version.as_ref().map(ToString::to_string),
        Some("2.0.0".into())
    );
    let rendered = Prompt::fragment(
        &base,
        &PromptArgs::new()
            .with("agent_name", "demo-agent")
            .with("agent_description", "a test"),
    )
    .unwrap()
    .render();
    assert!(rendered.contains("as the demo plugin describes you"));
    assert_eq!(installed.prompts.len(), 1);

    let server = registry
        .get::<rusty_harness::harness::plugins::McpServerEntity>(&EntityId::versioned(
            "demo/echo",
            "1.0.0".parse().unwrap(),
        ))
        .expect("server");
    let rusty_harness::harness::plugins::McpServerSpec::Stdio { command, args, .. } = &server.spec
    else {
        panic!("stdio")
    };
    assert!(
        command.ends_with("tests/fixtures/plugins/demo/bin/echo-mcp"),
        "PLUGIN_ROOT substituted: {command}"
    );
    assert_eq!(
        args[1], "/tmp/rh-data/plugins/demo",
        "PLUGIN_DATA substituted"
    );
    assert!(
        registry
            .get::<rusty_harness::harness::plugins::DeclaredHook>(&EntityId::versioned(
                "demo/audit",
                "1.0.0".parse().unwrap()
            ))
            .is_ok()
    );
    assert!(
        registry
            .get::<rusty_harness::harness::plugins::DeclaredCommand>(&EntityId::versioned(
                "plan",
                "1.0.0".parse().unwrap()
            ))
            .is_ok()
    );

    // A second install of the same plugin collides on every identity and registers nothing new.
    let again = discover_demo().await;
    let err = PluginLoader::new("/tmp/rh-data")
        .install(&again, &registry, &mut prompts)
        .expect_err("conflict");
    assert!(matches!(err, PluginError::Conflict { .. }), "{err}");
    assert!(err.to_string().contains("Plugin("));
    assert_eq!(registry.list::<Skill>().len(), 1);
}

/// Scenario: A programmatic plugin registers identically to one from the file system
#[tokio::test]
async fn a_programmatic_plugin_registers_identically() {
    let mut spec = discover_demo().await;
    spec.root = PluginRoot::InMemory;
    let registry = Registry::new();
    let mut prompts = PromptRegistry::builtin();
    PluginLoader::new("/tmp/rh-data")
        .install(&spec, &registry, &mut prompts)
        .expect("installs");
    let skill = registry.list::<Skill>().into_iter().next().expect("skill");
    let body = skill
        .body(&MemFs::default())
        .await
        .expect("body from memory");
    assert!(body.contains("Say hello"));
    let _ = Host::testing();
}

/// A file system that counts reads, delegating to the real one.
#[derive(Default)]
struct CountingFs {
    reads: std::sync::atomic::AtomicUsize,
}

impl CountingFs {
    fn reads(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl FileSystem for CountingFs {
    async fn read(&self, path: &Path) -> Result<bytes::Bytes, rusty_harness::host::HostError> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        TokioFs.read(path).await
    }
    async fn write_atomic(
        &self,
        path: &Path,
        contents: bytes::Bytes,
    ) -> Result<(), rusty_harness::host::HostError> {
        TokioFs.write_atomic(path, contents).await
    }
    async fn list(&self, dir: &Path) -> Result<Vec<PathBuf>, rusty_harness::host::HostError> {
        TokioFs.list(dir).await
    }
}

/// Scenario: A configured plugin path that cannot be read fails discovery
///     Given `[plugins] paths` naming the fixture directory and a directory that does not exist
///     When the plugins are discovered
///     Then the fixture's plugins are found
///     And discovery reports an error naming `paths` and the missing directory
///     So a harness never starts silently without the plugins its operator configured
#[tokio::test]
async fn a_configured_plugin_path_that_cannot_be_read_fails_discovery() {
    let missing = fixtures().join("no-such-directory");
    let loader = PluginLoader::new("/tmp/rh-data");
    let (specs, errors) = loader
        .discover(&[fixtures(), missing.clone()], &TokioFs)
        .await;
    assert!(specs.iter().any(|s| s.manifest.name == "demo"));
    let [PluginError::Invalid { field, reason, .. }] = errors.as_slice() else {
        panic!("one error for the missing path: {errors:?}");
    };
    assert_eq!(field, "paths");
    assert!(reason.contains("no-such-directory"), "{reason}");
}

//! Feature: The CLI
//! Requirement: docs/specs/issue-3/requirements.md#R27
//!
//! Scenario: The first run writes the configuration and reports what it did (T6)
//!     Given no configuration file under HOME and a model configured through the environment
//!     When `rusty-harness` starts with `NO_COLOR` set
//!     Then it writes `~/.rusty-harness/config.toml` with the commented defaults
//!     And prints the first-run lines from the string catalog, which are snapshotted
#![cfg(feature = "cli")]

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn normalize(line: &str, home: &Path) -> String {
    let port = regex_lite_port(line);
    let mut out = line.replace(&home.display().to_string(), "<home>");
    if let Some(p) = port {
        out = out.replace(&p, "<port>");
    }
    out
}

/// The port in `127.0.0.1:<port>`, without a regex crate.
fn regex_lite_port(line: &str) -> Option<String> {
    let idx = line.find("127.0.0.1:")?;
    let rest = &line[idx + "127.0.0.1:".len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

#[test]
fn snapshot_first_run_output() {
    let home =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("first-run-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rusty-harness"))
        .args(["--bind", "127.0.0.1:0", "--no-tui"])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &home)
        .env("NO_COLOR", "1")
        .env("LANG", "en_US.UTF-8")
        .env("RH__MODELS__DEFAULT__PROVIDER", "openai")
        .env("RH__MODELS__DEFAULT__MODEL", "mock")
        .env("RH__MODELS__DEFAULT__API_KEY", "env:RH_SNAPSHOT_KEY")
        .env("RH__MODELS__DEFAULT__BASE_URL", "http://127.0.0.1:9/v1")
        .env("RH_SNAPSHOT_KEY", "sk-snapshot")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let stdout = child.stdout.take().unwrap();
    let mut lines = Vec::new();
    let started = Instant::now();
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap();
        let done = line.contains("listening");
        lines.push(normalize(&line, &home));
        if done || started.elapsed() > Duration::from_secs(60) {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let config = home.join(".rusty-harness").join("config.toml");
    assert!(config.exists(), "the first run wrote the configuration");
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        rusty_harness::config::DEFAULT_CONFIG_TOML
    );

    let mut text = lines.join("\n");
    text.push('\n');
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/first_run_output.txt");
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let published = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        published, text,
        "first_run_output.txt is stale: rerun with UPDATE_SNAPSHOTS=1"
    );
    assert!(
        text.contains("wrote <home>/.rusty-harness/config.toml with commented defaults"),
        "{text}"
    );
    assert!(
        text.contains("models.default.api_key found in environment"),
        "{text}"
    );
    assert!(text.contains("loopback only"), "{text}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_configured_credential_that_does_not_resolve_refuses_to_start() {
    let home =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("no-secret-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rusty-harness"))
        .args(["--bind", "127.0.0.1:0"])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &home)
        .env("NO_COLOR", "1")
        .env("RH__MODELS__DEFAULT__PROVIDER", "openai")
        .env("RH__MODELS__DEFAULT__MODEL", "mock")
        .env("RH__MODELS__DEFAULT__API_KEY", "env:RH_MISSING_KEY")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(
        err.contains("credential reference env:RH_MISSING_KEY could not be resolved"),
        "{err}"
    );
    assert!(!err.contains("sk-"), "never a value");
    let _ = std::fs::remove_dir_all(&home);
}

/// Scenario: The demo configuration loads as checked in
///     Given `tests/fixtures/demo/config.toml`
///     When it is loaded with no environment
///     Then it names the demo model by reference, `none` durability and the demo plugin dir
#[tokio::test]
async fn the_demo_fixture_config_loads() {
    use rusty_harness::config::{Config, ConfigSources, DurableMode};
    use rusty_harness::host::TokioFs;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/demo/config.toml");
    let config = Config::load(
        &ConfigSources {
            file: Some(path),
            env: false,
            overrides: None,
        },
        &TokioFs,
    )
    .await
    .expect("the demo config loads");
    assert_eq!(config.durable.mode, DurableMode::None);
    assert_eq!(config.models["default"].model, "gpt-6.1");
    assert_eq!(
        config
            .secret_refs()
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
        ["models.default.api_key"]
    );
    assert!(
        config.plugins.paths.iter().any(|p| p.ends_with("plugins")),
        "{:?}",
        config.plugins.paths
    );
}

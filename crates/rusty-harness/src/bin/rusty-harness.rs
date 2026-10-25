//! The `rusty-harness` command (R27.1–2, R27.8, R30.1): one binary that starts the
//! harness, serves A2A and, from milestone 5, opens the terminal UI.
//!
//! `rusty-harness [--config <path>] [--url <a2a-url>] [--no-tui] [--locale <tag>] [--bind <addr>]`.
//! Precedence for the configuration file: `--config`, then `~/.rusty-harness/config.toml`
//! (written with commented defaults on the first run), then the built-in defaults.
//! Everything printed for a person comes from the string catalog in the resolved locale:
//! `--locale`, then `[locale]`, then `LANG`, then `en`.

use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};

use clap::Parser;
use rusty_harness::HarnessBuilder;
use rusty_harness::app_service::a2a_server::serve;
use rusty_harness::app_service::o11y;
use rusty_harness::assets::{Catalog, FmtArgs, keys};
use rusty_harness::config::{Config, ConfigSources, DurableMode};
use rusty_harness::harness::commands::Command;
use rusty_harness::harness::core_loop::agent::Agent as _;
use rusty_harness::harness::models::llm::{Llm, Modality};
use rusty_harness::host::{FileSystem, Host, TokioFs};
use rusty_harness::interaction::renderer::tui::{
    App, Footer, ModelEntry, ModelTrait, Theme, TuiOptions,
};
use rusty_harness::registry::Registry;

/// The command line.
#[derive(Debug, Parser)]
#[command(name = "rusty-harness", version, about = about(), disable_help_flag = false)]
struct Cli {
    /// The configuration file.
    #[arg(long, value_name = "path", help = arg_help(keys::cli::arg::CONFIG))]
    config: Option<PathBuf>,
    /// A remote harness to talk to.
    #[arg(long, value_name = "a2a-url", help = arg_help(keys::cli::arg::URL))]
    url: Option<url::Url>,
    /// No terminal UI.
    #[arg(long, help = arg_help(keys::cli::arg::NO_TUI))]
    no_tui: bool,
    /// The locale.
    #[arg(long, value_name = "tag", help = arg_help(keys::cli::arg::LOCALE))]
    locale: Option<String>,
    /// The bind address.
    #[arg(long, value_name = "addr", help = arg_help(keys::cli::arg::BIND))]
    bind: Option<String>,
}

/// The locale help and about texts are resolved before the configuration exists, so
/// they use `LANG` then `en`.
fn early_locale() -> String {
    resolve_locale(None, None, std::env::var("LANG").ok().as_deref())
}

fn about() -> String {
    Catalog::builtin().get(keys::cli::ABOUT, &early_locale(), &FmtArgs::new())
}

fn arg_help(key: rusty_harness::assets::StringKey) -> String {
    Catalog::builtin().get(key, &early_locale(), &FmtArgs::new())
}

/// `--locale`, then the configured locale, then `LANG` (its language tag), then `en`
/// (R27.8).
fn resolve_locale(flag: Option<&str>, configured: Option<&str>, lang: Option<&str>) -> String {
    if let Some(l) = flag.filter(|l| !l.trim().is_empty()) {
        return l.trim().to_owned();
    }
    if let Some(l) = configured.filter(|l| !l.trim().is_empty()) {
        return l.trim().to_owned();
    }
    lang.and_then(|lang| {
        let tag = lang.split(['.', '@']).next()?.trim();
        (!tag.is_empty() && tag != "C" && tag != "POSIX").then(|| tag.replace('_', "-"))
    })
    .unwrap_or_else(|| "en".to_owned())
}

/// `--config`, then `<home>/.rusty-harness/config.toml` (R27.2).
fn resolve_config_path(flag: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    flag.map(Path::to_path_buf)
        .or_else(|| home.map(|h| h.join(".rusty-harness").join("config.toml")))
}

/// What the person sees: the catalog in the resolved locale, on standard output or
/// error. `NO_COLOR` is honoured by never colouring.
struct Say {
    catalog: Catalog,
    locale: String,
}

impl Say {
    fn line(&self, key: rusty_harness::assets::StringKey, args: &FmtArgs) {
        println!("{}", self.catalog.get(key, &self.locale, args));
    }
    fn error(&self, key: rusty_harness::assets::StringKey, args: &FmtArgs) {
        eprintln!("{}", self.catalog.get(key, &self.locale, args));
    }
}

fn main() {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = runtime.block_on(run(cli));
    std::process::exit(code);
}

#[allow(clippy::too_many_lines)]
async fn run(cli: Cli) -> i32 {
    let fs = TokioFs;
    let mut say = Say {
        catalog: Catalog::builtin(),
        locale: early_locale(),
    };
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path = resolve_config_path(cli.config.as_deref(), home.as_deref());

    // First run: the file is written with commented defaults before it is read (R27.2).
    let mut wrote = None;
    if let Some(path) = &path
        && fs.read(path).await.is_err()
    {
        if let Err(e) = Config::write_default(path, &fs).await {
            say.error(
                keys::cli::error::CONFIG,
                &FmtArgs::new().text("reason", e.to_string()),
            );
            return 2;
        }
        wrote = Some(path.clone());
    }
    let config = match Config::load(
        &ConfigSources {
            file: path.clone(),
            env: true,
            overrides: None,
        },
        &fs,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            say.error(
                keys::cli::error::CONFIG,
                &FmtArgs::new().text("reason", e.to_string()),
            );
            return 2;
        }
    };
    say.locale = resolve_locale(
        cli.locale.as_deref(),
        config.locale.as_deref(),
        std::env::var("LANG").ok().as_deref(),
    );
    if let Some(path) = wrote {
        say.line(
            keys::cli::first_run::WROTE_CONFIG,
            &FmtArgs::new().text("path", path.display().to_string()),
        );
    }

    // Logs and spans per `[o11y]` (R24.2, R24.4), before anything else runs.
    let host = Host::native();
    let o11y = match o11y::init(&config.o11y, &host, o11y::Scope::Global) {
        Ok(guard) => guard,
        Err(e) => {
            say.error(
                keys::cli::error::STARTUP,
                &FmtArgs::new().text("reason", e.to_string()),
            );
            return 2;
        }
    };

    // Startup checks (fail-closed list): every credential reference resolves, and a
    // durable mode this build cannot run refuses to start.
    let secrets = host.secrets().expect("the native host resolves secrets");
    for (name, reference) in config.secret_refs() {
        match secrets.resolve(&reference).await {
            Ok(_) => say.line(
                keys::cli::first_run::CREDENTIAL_FOUND,
                &FmtArgs::new().text("name", name),
            ),
            Err(e) => {
                say.error(
                    keys::cli::error::SECRET_UNRESOLVABLE,
                    &FmtArgs::new()
                        .text("reference", reference.to_string())
                        .text("reason", e.to_string()),
                );
                return 2;
            }
        }
    }
    match config.durable.mode {
        DurableMode::None => say.line(keys::cli::first_run::DURABLE_NONE, &FmtArgs::new()),
        DurableMode::Embedded => {
            say.line(keys::cli::first_run::DURABLE_EMBEDDED, &FmtArgs::new());
        }
        DurableMode::Remote => say.line(
            keys::cli::first_run::DURABLE_REMOTE,
            &FmtArgs::new().text(
                "address",
                config
                    .durable
                    .temporal
                    .address
                    .as_ref()
                    .map_or_else(|| "the environment profile".to_owned(), ToString::to_string),
            ),
        ),
    }
    if let Some(url) = cli.url.clone() {
        // Attach to a remote harness: the same TUI, no server of our own (R27.3).
        say.line(keys::cli::first_run::STARTING_TUI, &FmtArgs::new());
        let options = tui_options(&config, None, None, &say.locale);
        let code = run_tui(options, &url, &host, &say).await;
        o11y.flush();
        return code;
    }

    let loopback_only = config.server.security.is_none();
    let harness = match HarnessBuilder::new(config).host(host).build().await {
        Ok(h) => h,
        Err(rusty_harness::SdkError::DurableConnect(reason)) => {
            say.error(keys::cli::error::DURABLE_UNREACHABLE, &FmtArgs::new());
            say.error(
                keys::cli::error::STARTUP,
                &FmtArgs::new().text("reason", reason),
            );
            return 2;
        }
        Err(e) => {
            say.error(
                keys::cli::error::STARTUP,
                &FmtArgs::new().text("reason", e.to_string()),
            );
            return 2;
        }
    };
    let server = match serve(&harness, cli.bind.as_deref()).await {
        Ok(s) => s,
        Err(e) => {
            say.error(
                keys::cli::error::STARTUP,
                &FmtArgs::new().text("reason", e.to_string()),
            );
            return 2;
        }
    };
    if loopback_only {
        say.line(keys::cli::first_run::LOOPBACK_ONLY, &FmtArgs::new());
    }
    let url = format!("http://{}/", server.addr);
    say.line(
        keys::cli::first_run::AGENT_CARD,
        &FmtArgs::new().text("url", format!("{url}.well-known/agent-card.json")),
    );
    say.line(
        keys::cli::first_run::LISTENING,
        &FmtArgs::new().text("url", url.clone()),
    );
    // Log mode when asked, and whenever there is no terminal to draw on (a service
    // manager, a test harness, a pipe): the server then runs until interrupted.
    let log_mode =
        cli.no_tui || !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal();
    let code = if log_mode {
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        match server.run(shutdown).await {
            Ok(()) => 0,
            Err(e) => {
                say.error(
                    keys::cli::error::SERVE,
                    &FmtArgs::new().text("reason", e.to_string()),
                );
                1
            }
        }
    } else {
        // The TUI is an A2A client of the server it just started (R27.3).
        say.line(keys::cli::first_run::STARTING_TUI, &FmtArgs::new());
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(server.run(async move {
            let _ = stopped.await;
        }));
        let options = tui_options(
            harness.config(),
            Some(harness.registry()),
            Some(&harness.agent().card().name),
            &say.locale,
        );
        let code = match url.parse::<url::Url>() {
            Ok(url) => run_tui(options, &url, harness.host(), &say).await,
            Err(e) => {
                say.error(
                    keys::cli::error::STARTUP,
                    &FmtArgs::new().text("reason", e.to_string()),
                );
                2
            }
        };
        let _ = stop.send(());
        let _ = server_task.await;
        code
    };
    o11y.flush();
    code
}

/// Runs the terminal UI against `url`; a failure is reported from the catalog.
async fn run_tui(options: TuiOptions, url: &url::Url, host: &Host, say: &Say) -> i32 {
    match rusty_harness::interaction::renderer::tui::run(App::new(options), url, host).await {
        Ok(()) => 0,
        Err(e) => {
            say.error(
                keys::cli::error::TUI,
                &FmtArgs::new().text("reason", e.to_string()),
            );
            1
        }
    }
}

/// What the TUI shows besides the stream: the `/model` rows and the commands come from
/// the in-process registry (a remote harness's are not known here), the footer from
/// the configuration.
fn tui_options(
    config: &Config,
    registry: Option<&Registry>,
    agent: Option<&str>,
    locale: &str,
) -> TuiOptions {
    let current = if config.models.contains_key("default") {
        "default".to_owned()
    } else {
        config.models.keys().next().cloned().unwrap_or_default()
    };
    let models = registry.map_or_else(Vec::new, |registry| {
        config
            .models
            .iter()
            .map(|(alias, m)| {
                let provider = serde_json::to_value(m.provider)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                let mut traits = Vec::new();
                if let Ok(llm) = registry.latest::<dyn Llm>(alias) {
                    let caps = llm.capabilities();
                    if caps.tools {
                        traits.push(ModelTrait::Tools);
                    }
                    if caps.input_modalities.contains(&Modality::Image) {
                        traits.push(ModelTrait::Vision);
                    }
                    if caps.input_modalities.contains(&Modality::Audio) {
                        traits.push(ModelTrait::Voice);
                    }
                }
                ModelEntry {
                    alias: alias.clone(),
                    label: format!("{provider}/{}", m.model),
                    traits,
                }
            })
            .collect()
    });
    let commands = registry.map_or_else(
        || vec!["model".to_owned(), "effort".to_owned(), "exit".to_owned()],
        |registry| {
            registry
                .list::<dyn Command>()
                .iter()
                .map(|c| c.descriptor().name.clone())
                .collect()
        },
    );
    TuiOptions {
        agent: agent.map_or_else(|| "rusty-harness".to_owned(), str::to_owned),
        locale: locale.to_owned(),
        models,
        commands,
        footer: Footer {
            effort: config.models.get(&current).and_then(|m| m.effort.clone()),
            model: current,
            durable: serde_json::to_value(config.durable.mode)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
        },
        mono: Theme::from_env().is_mono(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, resolve_config_path, resolve_locale};
    use clap::Parser;
    use std::path::Path;

    #[test]
    fn arguments_parse_and_the_config_path_precedence_is_flag_then_home() {
        let cli = Cli::try_parse_from([
            "rusty-harness",
            "--config",
            "/etc/rh.toml",
            "--no-tui",
            "--locale",
            "fr",
            "--url",
            "http://127.0.0.1:7474/",
            "--bind",
            "127.0.0.1:0",
        ])
        .unwrap();
        assert!(cli.no_tui);
        assert_eq!(cli.locale.as_deref(), Some("fr"));
        assert_eq!(cli.bind.as_deref(), Some("127.0.0.1:0"));
        assert_eq!(
            resolve_config_path(cli.config.as_deref(), Some(Path::new("/home/sam"))),
            Some("/etc/rh.toml".into())
        );
        assert_eq!(
            resolve_config_path(None, Some(Path::new("/home/sam"))),
            Some("/home/sam/.rusty-harness/config.toml".into())
        );
        assert_eq!(
            resolve_config_path(None, None),
            None,
            "no home, no file: defaults only"
        );
        assert!(Cli::try_parse_from(["rusty-harness", "--url", "not a url"]).is_err());
        let help = Cli::try_parse_from(["rusty-harness", "--help"])
            .unwrap_err()
            .to_string();
        assert!(
            help.contains("blazing fast") && help.contains("Bind the server here"),
            "help from the catalog: {help}"
        );
    }

    #[test]
    fn the_locale_is_the_flag_then_the_configured_one_then_lang_then_en() {
        assert_eq!(
            resolve_locale(Some("de"), Some("fr"), Some("en_US.UTF-8")),
            "de"
        );
        assert_eq!(resolve_locale(None, Some("fr"), Some("en_US.UTF-8")), "fr");
        assert_eq!(resolve_locale(None, None, Some("pt_BR.UTF-8")), "pt-BR");
        assert_eq!(resolve_locale(None, None, Some("C.UTF-8")), "en");
        assert_eq!(resolve_locale(None, None, None), "en");
        assert_eq!(resolve_locale(Some(" "), None, None), "en");
    }
}

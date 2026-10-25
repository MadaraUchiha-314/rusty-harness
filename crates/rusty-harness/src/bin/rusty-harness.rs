//! The server binary (R30.1). Milestone 3 task 20 ships the minimum the conformance
//! suite needs: `--config <path>` and `--bind <addr>`; task 25 brings the full CLI.

use std::path::PathBuf;

use rusty_harness::HarnessBuilder;
use rusty_harness::app_service::a2a_server::serve;
use rusty_harness::config::{Config, ConfigSources};
use rusty_harness::host::{Host, TokioFs};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut config: Option<PathBuf> = None;
    let mut bind: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config = args.next().map(PathBuf::from),
            "--bind" => bind = args.next(),
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = runtime.block_on(async move {
        let sources = ConfigSources {
            file: config,
            env: true,
            overrides: None,
        };
        let config = match Config::load(&sources, &TokioFs).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("configuration: {e}");
                return 2;
            }
        };
        let harness = match HarnessBuilder::new(config)
            .host(Host::native())
            .build()
            .await
        {
            Ok(h) => h,
            Err(e) => {
                eprintln!("startup: {e}");
                return 2;
            }
        };
        let server = match serve(&harness, bind.as_deref()).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("serve: {e}");
                return 2;
            }
        };
        println!("rusty-harness listening on http://{}/", server.addr);
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        match server.run(shutdown).await {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("server: {e}");
                1
            }
        }
    });
    std::process::exit(code);
}

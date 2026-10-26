//! The demo plugin's MCP server: a tiny stdio server on the official SDK, used by the
//! tool tests (testing-plan T2). Tools: `echo`, `slow`, `big`, `write`, `count`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};

#[derive(Clone)]
struct Echo {
    writes: Arc<AtomicU64>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct Text {
    /// The text.
    text: String,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct Slow {
    /// The text.
    text: String,
    /// How long to wait first.
    ms: u64,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct Big {
    /// How many bytes to return.
    bytes: usize,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct Nothing {}

#[tool_router]
impl Echo {
    #[tool(description = "Echoes the text.", annotations(read_only_hint = true))]
    async fn echo(&self, Parameters(p): Parameters<Text>) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult::success(vec![ContentBlock::text(p.text)]))
    }

    #[tool(
        description = "Echoes the text after a delay.",
        annotations(read_only_hint = true)
    )]
    async fn slow(&self, Parameters(p): Parameters<Slow>) -> Result<CallToolResult, ErrorData> {
        tokio::time::sleep(Duration::from_millis(p.ms)).await;
        Ok(CallToolResult::success(vec![ContentBlock::text(p.text)]))
    }

    #[tool(
        description = "Returns that many bytes.",
        annotations(read_only_hint = true)
    )]
    async fn big(&self, Parameters(p): Parameters<Big>) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult::success(vec![ContentBlock::text(
            "x".repeat(p.bytes),
        )]))
    }

    #[tool(
        description = "Writes the text (a side effect).",
        annotations(destructive_hint = true)
    )]
    async fn write(&self, Parameters(p): Parameters<Text>) -> Result<CallToolResult, ErrorData> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "wrote {}",
            p.text
        ))]))
    }

    #[tool(
        description = "How many writes happened.",
        annotations(read_only_hint = true)
    )]
    async fn count(&self, Parameters(_): Parameters<Nothing>) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult::success(vec![ContentBlock::text(
            self.writes.load(Ordering::SeqCst).to_string(),
        )]))
    }
}

#[tool_handler]
#[allow(clippy::unused_async_trait_impl)] // the SDK macro generates the impl
impl ServerHandler for Echo {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("The demo plugin's echo server.")
    }
}

fn main() {
    // `--data <dir>` arrives from mcp.json; this server keeps nothing on disk.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let server = Echo {
            writes: Arc::new(AtomicU64::new(0)),
        };
        let running = server
            .serve(rmcp::transport::stdio())
            .await
            .expect("serve stdio");
        let _ = running.waiting().await;
    });
}

//! Tools from MCP servers over the official MCP Rust SDK (R8.1); native only, the browser
//! uses the host's transport.
//!
//! [`RmcpSession`] wraps an `rmcp` client service: `stdio` spawns the server with a
//! scrubbed environment (only `PATH` and what `mcp.json` lists, as the host's process
//! spawner does), `streamable-http` goes through `rmcp`'s reqwest client.

use std::borrow::Cow;
use std::collections::BTreeMap;

use rmcp::ServiceExt as _;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use serde_json::Value;

use super::{McpCallOutcome, McpSession, McpToolInfo, ToolError};
use crate::harness::plugins::McpServerSpec;
use crate::harness_async_trait;
use crate::host::scrubbed_env;

/// An `rmcp` client session.
pub struct RmcpSession {
    service: RunningService<RoleClient, ()>,
}

impl core::fmt::Debug for RmcpSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RmcpSession").finish_non_exhaustive()
    }
}

#[allow(clippy::needless_pass_by_value)] // used as `map_err(transport)`
fn transport(e: impl ToString) -> ToolError {
    ToolError::Transport {
        reason: e.to_string(),
    }
}

impl RmcpSession {
    /// Connects to `spec`.
    ///
    /// # Errors
    ///
    /// [`ToolError::Transport`] when the process cannot be spawned, the endpoint is bad
    /// or the handshake fails; the legacy SSE transport is refused.
    pub async fn connect(spec: &McpServerSpec) -> Result<Self, ToolError> {
        let service = match spec {
            McpServerSpec::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let mut cmd = tokio::process::Command::new(command);
                cmd.args(args).env_clear().envs(scrubbed_env(env.clone()));
                if let Some(cwd) = cwd {
                    cmd.current_dir(cwd);
                }
                let child = TokioChildProcess::new(cmd).map_err(transport)?;
                ().serve(child).await.map_err(transport)?
            }
            McpServerSpec::StreamableHttp { url, headers } => {
                if !headers.is_empty() {
                    return Err(transport(format!(
                        "{url}: extra headers on streamable HTTP are not supported yet"
                    )));
                }
                let http = StreamableHttpClientTransport::from_uri(url.as_str());
                ().serve(http).await.map_err(transport)?
            }
            McpServerSpec::Sse { url } => {
                return Err(transport(format!(
                    "{url}: the legacy SSE transport is not supported"
                )));
            }
        };
        Ok(Self { service })
    }

    /// Connects to a spec built from parts (tests, programmatic plugins).
    ///
    /// # Errors
    ///
    /// As [`RmcpSession::connect`].
    pub async fn stdio(
        command: impl Into<String>,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    ) -> Result<Self, ToolError> {
        Self::connect(&McpServerSpec::Stdio {
            command: command.into(),
            args,
            env,
            cwd: None,
        })
        .await
    }

    /// Closes the session and the server process.
    pub async fn close(self) {
        let _ = self.service.cancel().await;
    }
}

harness_async_trait! {
    impl McpSession for RmcpSession {
        async fn list_tools(&self) -> Result<Vec<McpToolInfo>, ToolError> {
            let tools = self.service.list_all_tools().await.map_err(transport)?;
            Ok(tools
                .into_iter()
                .map(|t| {
                    let hints = t.annotations.as_ref();
                    McpToolInfo {
                        name: t.name.to_string(),
                        description: t.description.map(Cow::into_owned).unwrap_or_default(),
                        input_schema: Value::Object((*t.input_schema).clone()),
                        idempotent: hints.and_then(|a| a.read_only_hint) == Some(true)
                            || hints.and_then(|a| a.idempotent_hint) == Some(true),
                    }
                })
                .collect())
        }

        async fn call_tool(&self, name: &str, arguments: Value) -> Result<McpCallOutcome, ToolError> {
            let mut params = CallToolRequestParams::new(name.to_owned());
            if let Some(object) = arguments.as_object() {
                params = params.with_arguments(object.clone());
            }
            let result = self.service.call_tool(params).await.map_err(transport)?;
            let content = result.structured_content.clone().unwrap_or_else(|| {
                Value::String(
                    result
                        .content
                        .iter()
                        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            });
            Ok(McpCallOutcome {
                content,
                is_error: result.is_error.unwrap_or(false),
            })
        }
    }
}

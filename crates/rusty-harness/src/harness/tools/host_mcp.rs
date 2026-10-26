//! MCP over the host's channel (Q14): a minimal client for `initialize`, `tools/list` and
//! `tools/call` on JSON-RPC messages the host frames. The browser's only MCP path, and
//! the native fallback when the `mcp` feature is off.

use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use futures::lock::Mutex;
use serde_json::{Value, json};

use super::{McpCallOutcome, McpSession, McpToolInfo, ToolError};
use crate::harness::plugins::McpServerSpec;
use crate::harness_async_trait;
use crate::host::{Command, McpChannel, McpTarget, McpTransport};

/// The protocol version this client speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// The host target for a plugin's server spec.
///
/// # Errors
///
/// [`ToolError::Transport`] for the unsupported legacy SSE transport or a bad URL.
pub fn target_of(spec: &McpServerSpec) -> Result<McpTarget, ToolError> {
    match spec {
        McpServerSpec::Stdio {
            command,
            args,
            env,
            cwd,
        } => Ok(McpTarget::Stdio(Command {
            program: command.into(),
            args: args.clone(),
            env: env.clone(),
            cwd: cwd.as_ref().map(Into::into),
        })),
        McpServerSpec::StreamableHttp { url, .. } => Ok(McpTarget::StreamableHttp {
            url: url.parse().map_err(|e| ToolError::Transport {
                reason: format!("{url}: {e}"),
            })?,
        }),
        McpServerSpec::Sse { url } => Err(ToolError::Transport {
            reason: format!("{url}: the legacy SSE transport is not supported"),
        }),
    }
}

/// A session over one host channel; requests are sequential.
pub struct ChannelSession {
    channel: Mutex<Box<dyn McpChannel>>,
    next_id: AtomicU64,
}

impl core::fmt::Debug for ChannelSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChannelSession").finish_non_exhaustive()
    }
}

#[allow(clippy::needless_pass_by_value)] // used as `map_err(transport)`
fn transport(e: impl ToString) -> ToolError {
    ToolError::Transport {
        reason: e.to_string(),
    }
}

impl ChannelSession {
    /// Connects through `transport` and performs the initialize handshake.
    ///
    /// # Errors
    ///
    /// [`ToolError::Transport`].
    pub async fn connect(
        transport: &dyn McpTransport,
        target: McpTarget,
    ) -> Result<Self, ToolError> {
        let channel = transport.connect(target).await.map_err(self::transport)?;
        let session = Self {
            channel: Mutex::new(channel),
            next_id: AtomicU64::new(1),
        };
        session
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "rusty-harness", "version": env!("CARGO_PKG_VERSION") }
                }),
            )
            .await?;
        session
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(session)
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), ToolError> {
        let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let mut channel = self.channel.lock().await;
        channel
            .send(Bytes::from(message.to_string()))
            .await
            .map_err(transport)
    }

    /// One request; replies to other ids and notifications are skipped.
    async fn request(&self, method: &str, params: Value) -> Result<Value, ToolError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut channel = self.channel.lock().await;
        channel
            .send(Bytes::from(message.to_string()))
            .await
            .map_err(transport)?;
        loop {
            let Some(next) = channel.recv().await else {
                return Err(transport("channel closed"));
            };
            let bytes = next.map_err(transport)?;
            let value: Value = serde_json::from_slice(&bytes).map_err(transport)?;
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(ToolError::Transport {
                    reason: format!(
                        "{method}: {}",
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("error")
                    ),
                });
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

/// Decodes a `tools/call` result in the wire shape (shared with the `rmcp` session's
/// own decoding only in spirit: that one uses the SDK's types).
pub(crate) fn outcome_of(result: &Value) -> McpCallOutcome {
    let content = result.get("structuredContent").cloned().unwrap_or_else(|| {
        let texts: Vec<&str> = result
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        Value::String(texts.join("\n"))
    });
    McpCallOutcome {
        content,
        is_error: result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

harness_async_trait! {
    impl McpSession for ChannelSession {
        async fn list_tools(&self) -> Result<Vec<McpToolInfo>, ToolError> {
            let mut tools = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let params = cursor.as_ref().map_or_else(|| json!({}), |c| json!({ "cursor": c }));
                let page = self.request("tools/list", params).await?;
                for tool in page.get("tools").and_then(Value::as_array).into_iter().flatten() {
                    let annotations = tool.get("annotations");
                    let hint = |name: &str| annotations.and_then(|a| a.get(name)).and_then(Value::as_bool) == Some(true);
                    tools.push(McpToolInfo {
                        name: tool.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
                        description: tool.get("description").and_then(Value::as_str).unwrap_or_default().to_owned(),
                        input_schema: tool.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
                        idempotent: hint("readOnlyHint") || hint("idempotentHint"),
                    });
                }
                cursor = page.get("nextCursor").and_then(Value::as_str).map(str::to_owned);
                if cursor.is_none() {
                    return Ok(tools);
                }
            }
        }

        async fn call_tool(&self, name: &str, arguments: Value) -> Result<McpCallOutcome, ToolError> {
            let result = self.request("tools/call", json!({ "name": name, "arguments": arguments })).await?;
            Ok(outcome_of(&result))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ChannelSession, outcome_of, target_of};
    use crate::harness::plugins::McpServerSpec;
    use crate::harness::tools::McpSession as _;
    use crate::harness_async_trait;
    use crate::host::{HostError, McpChannel, McpTarget, McpTransport};
    use bytes::Bytes;
    use serde_json::{Value, json};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    /// A scripted server: answers each request from its id and method.
    struct Fake {
        log: Arc<Mutex<Vec<Value>>>,
        queue: VecDeque<Bytes>,
    }

    harness_async_trait! {
        impl McpChannel for Fake {
            async fn send(&mut self, message: Bytes) -> Result<(), HostError> {
                let msg: Value = serde_json::from_slice(&message).unwrap();
                self.log.lock().unwrap().push(msg.clone());
                let Some(id) = msg.get("id") else { return Ok(()) };
                let reply = match msg["method"].as_str().unwrap() {
                    "initialize" => json!({ "protocolVersion": "2025-06-18", "capabilities": { "tools": {} }, "serverInfo": { "name": "fake", "version": "0" } }),
                    "tools/list" if msg["params"].get("cursor").is_none() => json!({ "tools": [{ "name": "echo", "description": "Echoes", "inputSchema": { "type": "object" }, "annotations": { "readOnlyHint": true } }], "nextCursor": "p2" }),
                    "tools/list" => json!({ "tools": [{ "name": "write", "inputSchema": { "type": "object" } }] }),
                    "tools/call" if msg["params"]["name"] == "echo" => json!({ "content": [{ "type": "text", "text": msg["params"]["arguments"]["text"] }] }),
                    _ => json!({ "content": [{ "type": "text", "text": "nope" }], "isError": true }),
                };
                // A stray notification first, to be skipped.
                self.queue.push_back(Bytes::from(json!({ "jsonrpc": "2.0", "method": "notifications/progress" }).to_string()));
                self.queue.push_back(Bytes::from(json!({ "jsonrpc": "2.0", "id": id, "result": reply }).to_string()));
                Ok(())
            }
            async fn recv(&mut self) -> Option<Result<Bytes, HostError>> {
                self.queue.pop_front().map(Ok)
            }
        }
    }

    struct FakeTransport(Arc<Mutex<Vec<Value>>>);
    harness_async_trait! {
        impl McpTransport for FakeTransport {
            async fn connect(&self, _target: McpTarget) -> Result<Box<dyn McpChannel>, HostError> {
                Ok(Box::new(Fake { log: Arc::clone(&self.0), queue: VecDeque::new() }))
            }
        }
    }

    #[tokio::test]
    async fn the_host_channel_client_initializes_pages_tool_lists_and_calls_tools() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let spec = McpServerSpec::StreamableHttp {
            url: "https://mcp.example/rpc".into(),
            headers: std::collections::BTreeMap::default(),
        };
        let session =
            ChannelSession::connect(&FakeTransport(Arc::clone(&log)), target_of(&spec).unwrap())
                .await
                .unwrap();
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .map(|m| m["method"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
            ["initialize", "notifications/initialized"]
        );
        let tools = session.list_tools().await.unwrap();
        assert_eq!(
            tools
                .iter()
                .map(|t| (t.name.as_str(), t.idempotent))
                .collect::<Vec<_>>(),
            [("echo", true), ("write", false)]
        );
        let ok = session
            .call_tool("echo", json!({ "text": "hi" }))
            .await
            .unwrap();
        assert_eq!(
            (ok.content, ok.is_error),
            (Value::String("hi".into()), false)
        );
        let err = session.call_tool("write", json!({})).await.unwrap();
        assert!(err.is_error);
        assert!(
            target_of(&McpServerSpec::Sse {
                url: "https://x".into()
            })
            .is_err()
        );
        assert_eq!(
            outcome_of(&json!({ "structuredContent": { "n": 1 } })).content,
            json!({ "n": 1 })
        );
    }
}

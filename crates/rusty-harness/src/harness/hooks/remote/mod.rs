//! Remote hooks over JSON-RPC and MCP with the same typed context as local hooks (R4.10).
//!
//! A remote hook is an ordinary hook whose body lives in another process. Two shapes,
//! sherma's two: a JSON-RPC 2.0 endpoint (`POST`, method `hook.<phase>.<point>`) and a
//! tool on an MCP server (`hook_<phase>_<point>`). Both receive `{ cx, input }` (and the
//! `output` for `post`) in the operation's serde form and answer with the same outcome
//! types a local hook returns, so the dispatcher cannot tell them apart (R1.3).

use core::marker::PhantomData;
use std::sync::Arc;

use bytes::Bytes;
use serde::{Deserialize, Serialize};

use super::{
    GuardHook, GuardOutcome, HookCx, HookError, HookPoint, InHook, Next, Operation, Phase,
    PostHook, PreHook, PreOutcome, Veto,
};
use crate::error::HarnessError;
use crate::harness_async_trait;
use crate::host::{HttpClient, HttpRequest};
use crate::registry::EntityId;

/// The part of a [`HookCx`] a remote hook can see: identities, never handles.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCx {
    /// The task, when the operation belongs to one.
    pub task: Option<String>,
    /// The A2A context id, when known.
    pub context_id: Option<String>,
    /// The W3C `traceparent`, so the remote side joins the trace.
    pub traceparent: Option<String>,
}

impl From<&HookCx> for RemoteCx {
    fn from(cx: &HookCx) -> Self {
        Self {
            task: cx.task.clone(),
            context_id: cx.context_id.clone(),
            traceparent: cx.traceparent.clone(),
        }
    }
}

/// What a remote `pre` hook answers.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemotePreOutcome<I> {
    /// Continue with this input.
    Continue(I),
    /// Do not run the operation.
    Veto(Veto),
}

/// What a remote guard answers.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteGuardOutcome {
    /// Dispatch may proceed.
    Allow,
    /// Dispatch must not proceed.
    Deny(Veto),
}

/// What a remote `in` hook answers: it cannot call `next` across a process boundary, so
/// it either replaces the operation or hands back the (possibly rewritten) input.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteInOutcome<I, O> {
    /// Continue the chain with this input.
    Next(I),
    /// The operation is done; this is its output.
    Output(O),
}

/// What a remote `post` hook answers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemotePostOutcome<O> {
    /// The (possibly rewritten) output.
    pub output: O,
}

/// Calls a tool on an MCP server. Supplied by the MCP module once it exists (task 15);
/// until then by whoever constructs the hook.
pub trait McpToolCaller: Send + Sync {
    /// Calls `tool` with `arguments` and returns its structured result.
    fn call_tool(
        &self,
        tool: &str,
        arguments: serde_json::Value,
    ) -> crate::runtime::HarnessFuture<'_, Result<serde_json::Value, HookError>>;
}

/// Where the remote hook's body lives.
#[derive(Clone)]
pub enum RemoteTransport {
    /// A JSON-RPC 2.0 endpoint reached through the host's HTTP client.
    JsonRpc {
        /// The endpoint.
        url: url::Url,
        /// Extra headers (an `Authorization` value is set from a `Secret` per request by
        /// the caller that built the map, never stored elsewhere).
        headers: http::HeaderMap,
        /// The client.
        http: Arc<dyn HttpClient>,
    },
    /// A tool on an MCP server.
    McpTool {
        /// The server the tool lives on.
        server: EntityId,
        /// The caller for that server.
        caller: Arc<dyn McpToolCaller>,
    },
}

impl core::fmt::Debug for RemoteTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::JsonRpc { url, .. } => f
                .debug_struct("JsonRpc")
                .field("url", url)
                .finish_non_exhaustive(),
            Self::McpTool { server, .. } => f
                .debug_struct("McpTool")
                .field("server", server)
                .finish_non_exhaustive(),
        }
    }
}

/// A hook whose body runs in another process. Implements all four hook traits; register
/// it in whichever phase the plugin declared.
pub struct RemoteHook<O: Operation> {
    id: EntityId,
    transport: RemoteTransport,
    _op: PhantomData<fn() -> O>,
}

impl<O: Operation> core::fmt::Debug for RemoteHook<O> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RemoteHook")
            .field("id", &self.id)
            .field("transport", &self.transport)
            .finish()
    }
}

/// The JSON-RPC method name for a phase and point: `hook.pre.tool.execute`.
#[must_use]
pub fn method_name(phase: Phase, point: HookPoint) -> String {
    format!("hook.{}.{}", phase_name(phase), point.name())
}

/// The MCP tool name for a phase and point: `hook_pre_tool_execute`.
#[must_use]
pub fn tool_name(phase: Phase, point: HookPoint) -> String {
    format!(
        "hook_{}_{}",
        phase_name(phase),
        point.name().replace('.', "_")
    )
}

const fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Pre => "pre",
        Phase::Guard => "guard",
        Phase::In => "in",
        Phase::Post => "post",
    }
}

#[derive(Serialize)]
struct JsonRpcRequest<'a, P> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: P,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

/// Calls `method` (JSON-RPC) or `tool` (MCP) over `transport` with `params` and decodes
/// the result. Shared by remote hooks and plugin-declared commands.
///
/// # Errors
///
/// [`HookError::Remote`] for a transport, protocol or decoding failure.
pub async fn call_remote<P: Serialize + Send + Sync, R: serde::de::DeserializeOwned>(
    transport: &RemoteTransport,
    method: &str,
    tool: &str,
    params: P,
) -> Result<R, HookError> {
    let value = match transport {
        RemoteTransport::JsonRpc { url, headers, http } => {
            let body = serde_json::to_vec(&JsonRpcRequest {
                jsonrpc: "2.0",
                id: 1,
                method,
                params,
            })
            .map_err(|e| HookError::Remote(format!("encode: {e}")))?;
            let mut request = HttpRequest::new(http::Method::POST, url.clone());
            request.headers = headers.clone();
            request.headers.insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/json"),
            );
            request.body = Bytes::from(body);
            let response = http
                .send(request)
                .await
                .map_err(|e| HookError::Remote(e.to_string()))?;
            if !response.status.is_success() {
                return Err(HookError::Remote(format!(
                    "http {}",
                    response.status.as_u16()
                )));
            }
            let envelope: JsonRpcResponse = serde_json::from_slice(&response.body)
                .map_err(|e| HookError::Remote(format!("decode: {e}")))?;
            if let Some(err) = envelope.error {
                return Err(HookError::Remote(format!(
                    "json-rpc {}: {}",
                    err.code, err.message
                )));
            }
            envelope
                .result
                .ok_or_else(|| HookError::Remote("json-rpc response without result".into()))?
        }
        RemoteTransport::McpTool { caller, .. } => {
            let arguments = serde_json::to_value(params)
                .map_err(|e| HookError::Remote(format!("encode: {e}")))?;
            caller.call_tool(tool, arguments).await?
        }
    };
    serde_json::from_value(value).map_err(|e| HookError::Remote(format!("decode: {e}")))
}

impl<O: Operation> RemoteHook<O> {
    /// A remote hook for `O` reached through `transport`.
    #[must_use]
    pub fn new(id: EntityId, transport: RemoteTransport) -> Self {
        Self {
            id,
            transport,
            _op: PhantomData,
        }
    }

    /// The hook's id, as registered.
    #[must_use]
    pub fn id(&self) -> &EntityId {
        &self.id
    }

    async fn call<P: Serialize + Send + Sync, R: serde::de::DeserializeOwned>(
        &self,
        phase: Phase,
        params: P,
    ) -> Result<R, HookError> {
        call_remote(
            &self.transport,
            &method_name(phase, O::POINT),
            &tool_name(phase, O::POINT),
            params,
        )
        .await
    }
}

#[derive(Serialize)]
struct PreParams<'a, I> {
    cx: RemoteCx,
    input: &'a I,
}

#[derive(Serialize)]
struct PostParams<'a, I, R> {
    cx: RemoteCx,
    input: &'a I,
    output: &'a R,
}

harness_async_trait! {
    impl<O: Operation> PreHook<O> for RemoteHook<O>
    where
        O::Input: Sync,
    {
        async fn pre(&self, cx: &HookCx, input: O::Input) -> Result<PreOutcome<O::Input>, HookError> {
            let outcome: RemotePreOutcome<O::Input> =
                self.call(Phase::Pre, PreParams { cx: RemoteCx::from(cx), input: &input }).await?;
            Ok(match outcome {
                RemotePreOutcome::Continue(next) => PreOutcome::Continue(next),
                RemotePreOutcome::Veto(veto) => PreOutcome::Veto(veto),
            })
        }
    }
}

harness_async_trait! {
    impl<O: Operation> GuardHook<O> for RemoteHook<O>
    where
        O::Input: Sync,
    {
        async fn guard(&self, cx: &HookCx, input: &O::Input) -> Result<GuardOutcome, HookError> {
            let outcome: RemoteGuardOutcome =
                self.call(Phase::Guard, PreParams { cx: RemoteCx::from(cx), input }).await?;
            Ok(match outcome {
                RemoteGuardOutcome::Allow => GuardOutcome::Allow,
                RemoteGuardOutcome::Deny(veto) => GuardOutcome::Deny(veto),
            })
        }
    }
}

harness_async_trait! {
    impl<O: Operation> InHook<O> for RemoteHook<O>
    where
        O::Input: Sync,
        O::Output: Sync,
    {
        async fn run<'a>(&self, cx: &'a HookCx, input: O::Input, next: Next<'a, O>) -> Result<O::Output, HarnessError> {
            let outcome: RemoteInOutcome<O::Input, O::Output> = self
                .call(Phase::In, PreParams { cx: RemoteCx::from(cx), input: &input })
                .await
                .map_err(|source| HarnessError::Hook {
                    point: O::POINT,
                    phase: Phase::In,
                    source,
                    policy: super::FailurePolicy::FailClosed,
                })?;
            match outcome {
                RemoteInOutcome::Next(input) => next(input).await,
                RemoteInOutcome::Output(output) => Ok(output),
            }
        }
    }
}

harness_async_trait! {
    impl<O: Operation> PostHook<O> for RemoteHook<O>
    where
        O::Input: Sync,
        O::Output: Sync,
    {
        async fn post(&self, cx: &HookCx, input: &O::Input, output: O::Output) -> Result<O::Output, HookError> {
            let outcome: RemotePostOutcome<O::Output> =
                self.call(Phase::Post, PostParams { cx: RemoteCx::from(cx), input, output: &output }).await?;
            Ok(outcome.output)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{McpToolCaller, RemoteHook, RemoteTransport, method_name, tool_name};
    use crate::harness::hooks::{
        Hook, HookCx, HookDispatcher, HookError, HookOrder, HookPoint, Operation, Phase,
    };
    use crate::host::testing::MockHttp;
    use crate::host::{Host, HttpResponse};
    use crate::registry::{EntityId, Registry};
    use crate::runtime::HarnessFuture;
    use std::sync::{Arc, Mutex};

    struct Exec;
    impl Operation for Exec {
        const POINT: HookPoint = HookPoint::ToolExecute;
        type Input = String;
        type Output = String;
        fn identity(input: &String) -> String {
            input.split_whitespace().next().unwrap_or("").to_owned()
        }
    }

    /// A JSON-RPC "server" that records the methods and params it saw and answers like a
    /// local hook would: pre appends a tag, guard denies `shell`, post uppercases.
    fn rpc_server(seen: Arc<Mutex<Vec<(String, serde_json::Value)>>>) -> MockHttp {
        MockHttp::new(move |req| {
            let body: serde_json::Value = serde_json::from_slice(&req.body).expect("json-rpc body");
            let method = body["method"].as_str().expect("method").to_owned();
            let params = body["params"].clone();
            seen.lock().unwrap().push((method.clone(), params.clone()));
            let result = match method.as_str() {
                "hook.pre.tool.execute" => {
                    serde_json::json!({ "continue": format!("{} +rpc", params["input"].as_str().unwrap()) })
                }
                "hook.guard.tool.execute" => {
                    if params["input"].as_str().unwrap().starts_with("shell") {
                        serde_json::json!({ "deny": { "reason": "tool.denied", "detail": null, "by": { "id": "rpc-policy", "version": null } } })
                    } else {
                        serde_json::json!("allow")
                    }
                }
                "hook.post.tool.execute" => {
                    serde_json::json!({ "output": params["output"].as_str().unwrap().to_uppercase() })
                }
                other => panic!("unexpected method {other}"),
            };
            let envelope =
                serde_json::json!({ "jsonrpc": "2.0", "id": body["id"], "result": result });
            Ok(HttpResponse {
                status: http::StatusCode::OK,
                headers: http::HeaderMap::new(),
                body: serde_json::to_vec(&envelope).unwrap().into(),
            })
        })
    }

    /// An MCP "server" exposing the same hook as a tool.
    struct McpServer(Arc<Mutex<Vec<(String, serde_json::Value)>>>);
    impl McpToolCaller for McpServer {
        fn call_tool(
            &self,
            tool: &str,
            arguments: serde_json::Value,
        ) -> HarnessFuture<'_, Result<serde_json::Value, HookError>> {
            self.0
                .lock()
                .unwrap()
                .push((tool.to_owned(), arguments.clone()));
            let out = match tool {
                "hook_pre_tool_execute" => {
                    serde_json::json!({ "continue": format!("{} +mcp", arguments["input"].as_str().unwrap()) })
                }
                "hook_in_tool_execute" => serde_json::json!({ "next": arguments["input"] }),
                other => panic!("unexpected tool {other}"),
            };
            Box::pin(async move { Ok(out) })
        }
    }

    fn cx() -> HookCx {
        let mut cx = HookCx::new(Arc::new(Registry::new()), Arc::new(Host::testing()));
        cx.task = Some("task_1".into());
        cx.traceparent = Some("00-abc-def-01".into());
        cx
    }

    async fn echo(input: String) -> Result<String, crate::HarnessError> {
        Ok(format!("ran[{input}]"))
    }

    #[test]
    fn method_and_tool_names_follow_sherma_shapes() {
        assert_eq!(
            method_name(Phase::Pre, HookPoint::ModelCall),
            "hook.pre.model.call"
        );
        assert_eq!(
            tool_name(Phase::Post, HookPoint::ToolResultSize),
            "hook_post_tool_result_size"
        );
    }

    /// Feature: Remote hooks
    /// Requirement: docs/specs/issue-3/requirements.md#R4
    ///
    /// Scenario: Remote hooks over JSON-RPC and MCP see the same typed context as local hooks
    ///     Given a JSON-RPC hook server and an MCP hook server for `tool.execute`
    ///     When the dispatcher runs a tool call through a remote pre, guard and post hook
    ///     Then each receives the task id, the traceparent and the typed input
    ///     And their answers rewrite, deny and rewrite exactly as local hooks would
    #[tokio::test]
    async fn remote_hooks_over_json_rpc_and_mcp_see_the_same_typed_context_as_local_hooks() {
        let seen_rpc = Arc::new(Mutex::new(Vec::new()));
        let seen_mcp = Arc::new(Mutex::new(Vec::new()));
        let http: Arc<dyn crate::host::HttpClient> = Arc::new(rpc_server(seen_rpc.clone()));
        let rpc = || RemoteTransport::JsonRpc {
            url: "https://hooks.example/rpc".parse().unwrap(),
            headers: http::HeaderMap::new(),
            http: http.clone(),
        };
        let mcp = RemoteTransport::McpTool {
            server: EntityId::new("hooks-mcp"),
            caller: Arc::new(McpServer(seen_mcp.clone())),
        };

        let d = HookDispatcher::new();
        d.register::<Exec>(
            EntityId::new("rpc-pre"),
            Hook::Pre(Arc::new(RemoteHook::<Exec>::new(
                EntityId::new("rpc-pre"),
                rpc(),
            ))),
            HookOrder::default(),
        );
        d.register::<Exec>(
            EntityId::new("mcp-pre"),
            Hook::Pre(Arc::new(RemoteHook::<Exec>::new(
                EntityId::new("mcp-pre"),
                mcp.clone(),
            ))),
            HookOrder::default(),
        );
        d.register::<Exec>(
            EntityId::new("rpc-guard"),
            Hook::Guard(Arc::new(RemoteHook::<Exec>::new(
                EntityId::new("rpc-guard"),
                rpc(),
            ))),
            HookOrder::default(),
        );
        d.register::<Exec>(
            EntityId::new("mcp-in"),
            Hook::In(Arc::new(RemoteHook::<Exec>::new(
                EntityId::new("mcp-in"),
                mcp,
            ))),
            HookOrder::default(),
        );
        d.register::<Exec>(
            EntityId::new("rpc-post"),
            Hook::Post(Arc::new(RemoteHook::<Exec>::new(
                EntityId::new("rpc-post"),
                rpc(),
            ))),
            HookOrder::default(),
        );

        let out = d
            .run::<Exec, _, _>(&cx(), "echo hi".into(), echo)
            .await
            .expect("runs");
        assert_eq!(out, "RAN[ECHO HI +RPC +MCP]");

        let rpc_calls = seen_rpc.lock().unwrap().clone();
        assert_eq!(
            rpc_calls
                .iter()
                .map(|(m, _)| m.as_str())
                .collect::<Vec<_>>(),
            [
                "hook.pre.tool.execute",
                "hook.guard.tool.execute",
                "hook.post.tool.execute"
            ]
        );
        for (_, params) in &rpc_calls {
            assert_eq!(params["cx"]["task"], "task_1");
            assert_eq!(params["cx"]["traceparent"], "00-abc-def-01");
        }
        let mcp_calls = seen_mcp.lock().unwrap().clone();
        assert_eq!(
            mcp_calls
                .iter()
                .map(|(m, _)| m.as_str())
                .collect::<Vec<_>>(),
            ["hook_pre_tool_execute", "hook_in_tool_execute"]
        );
        assert_eq!(mcp_calls[0].1["cx"]["task"], "task_1");

        let err = d
            .run::<Exec, _, _>(&cx(), "shell rm".into(), echo)
            .await
            .expect_err("remote guard denies");
        assert!(
            matches!(err, crate::HarnessError::Vetoed(v) if v.reason == "tool.denied" && v.by == EntityId::new("rpc-policy"))
        );
    }

    #[tokio::test]
    async fn a_remote_failure_is_a_remote_hook_error() {
        let http: Arc<dyn crate::host::HttpClient> =
            Arc::new(MockHttp::always(http::StatusCode::BAD_GATEWAY, ""));
        let hook = RemoteHook::<Exec>::new(
            EntityId::new("down"),
            RemoteTransport::JsonRpc {
                url: "https://hooks.example/rpc".parse().unwrap(),
                headers: http::HeaderMap::new(),
                http,
            },
        );
        let err = crate::harness::hooks::PreHook::<Exec>::pre(&hook, &cx(), "echo".into())
            .await
            .expect_err("down");
        assert!(matches!(err, HookError::Remote(m) if m.contains("502")));
    }
}

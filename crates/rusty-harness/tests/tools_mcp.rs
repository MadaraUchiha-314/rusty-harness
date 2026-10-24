//! Feature: Tools and MCP
//! Requirement: docs/specs/issue-3/requirements.md#R8
//!
//! The T2 scenarios of task 15 against the fixture stdio MCP server (`echo-mcp`, built
//! from `tests/fixtures/plugins/demo/bin/echo-mcp.rs` with the `mcp` feature).
#![cfg(all(feature = "mcp", not(target_arch = "wasm32")))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use rusty_harness::harness::core_loop::task::{Attempt, Caller, Role};
use rusty_harness::harness::hooks::{Hook, HookCx, HookDispatcher, HookOrder};
use rusty_harness::harness::tools::mcp::RmcpSession;
use rusty_harness::harness::tools::{
    CancelToken, Executed, McpServer, OversizePolicy, Tool, ToolCall, ToolError, ToolExecute,
    ToolExecutor, ToolPolicy, ToolResult,
};
use rusty_harness::host::Host;
use rusty_harness::registry::{EntityId, Registry};
use serde_json::json;

struct Rig {
    executor: ToolExecutor,
    dispatcher: Arc<HookDispatcher>,
    cx: HookCx,
    server: McpServer,
}

async fn rig(limit: usize) -> Rig {
    let session = RmcpSession::stdio(
        env!("CARGO_BIN_EXE_echo-mcp"),
        vec!["--data".into(), "/tmp/rh-data".into()],
        std::collections::BTreeMap::default(),
    )
    .await
    .expect("the fixture server starts");
    let server = McpServer::new(
        EntityId::versioned("demo/echo", "1.0.0".parse().unwrap()),
        Arc::new(session),
    );
    let registry = Arc::new(Registry::new());
    let ids = server
        .register_tools(&registry)
        .await
        .expect("tools listed");
    assert_eq!(ids.len(), 5, "{ids:?}");
    assert!(ids.iter().any(|id| id.id.as_ref() == "demo/echo/echo"));
    let dispatcher = Arc::new(HookDispatcher::new());
    let host = Arc::new(Host::testing());
    let cx = HookCx::new(Arc::clone(&registry), Arc::clone(&host));
    let executor = ToolExecutor::new(Arc::clone(&registry), Arc::clone(&dispatcher), host)
        .with_result_limit(limit, OversizePolicy::Truncate);
    Rig {
        executor,
        dispatcher,
        cx,
        server,
    }
}

fn call(id: &str, name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: format!("demo/echo/{name}"),
        arguments: args,
    }
}

fn result(e: &Executed) -> &ToolResult {
    match e {
        Executed::Result { result } => result,
        Executed::Suspended { .. } => panic!("suspended"),
    }
}

async fn run(rig: &Rig, caller: &Caller, calls: Vec<ToolCall>) -> Vec<Executed> {
    rig.executor
        .execute_all(
            &rig.cx,
            &"t1".into(),
            Attempt { iteration: 1, n: 1 },
            caller,
            &CancelToken::new(),
            calls,
        )
        .await
}

/// Scenario: Several tool calls run concurrently and return in issue order
///     Given the fixture MCP server with a `slow` tool
///     When the model issues three calls, the first the slowest
///     Then they run concurrently and the results come back in issue order
#[tokio::test]
async fn several_tool_calls_run_concurrently_and_return_in_issue_order() {
    let rig = rig(64 * 1024).await;
    let admin = Caller::human("a", [Role::Admin]);
    let started = Instant::now();
    let out = run(
        &rig,
        &admin,
        vec![
            call("c1", "slow", json!({ "text": "first", "ms": 400 })),
            call("c2", "slow", json!({ "text": "second", "ms": 150 })),
            call("c3", "echo", json!({ "text": "third" })),
        ],
    )
    .await;
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(520),
        "concurrent, not serial: {elapsed:?}"
    );
    let texts: Vec<String> = out.iter().map(|e| result(e).render()).collect();
    assert_eq!(texts, ["first", "second", "third"]);
    assert_eq!(
        out.iter()
            .map(|e| result(e).call_id.as_str())
            .collect::<Vec<_>>(),
        ["c1", "c2", "c3"]
    );
}

/// Scenario: A tool result above the size limit is truncated by the configured policy
#[tokio::test]
async fn a_tool_result_above_the_size_limit_is_truncated_by_the_configured_policy() {
    let rig = rig(1024).await;
    let admin = Caller::human("a", [Role::Admin]);
    let out = run(
        &rig,
        &admin,
        vec![call("c1", "big", json!({ "bytes": 200_000 }))],
    )
    .await;
    let r = result(&out[0]);
    assert!(r.shortened);
    assert!(r.render().len() <= 1024);
    assert!(r.render().contains("bytes cut"));
}

/// Scenario: A pre hook veto stops tool execution and the model is told
#[tokio::test]
async fn a_pre_hook_veto_stops_tool_execution_and_the_model_is_told() {
    use rusty_harness::harness::hooks::{HookError, PreHook, PreOutcome, Veto};
    use rusty_harness::harness::tools::ToolInvocation;
    struct NoWrites;
    #[async_trait::async_trait]
    impl PreHook<ToolExecute> for NoWrites {
        async fn pre(
            &self,
            _cx: &HookCx,
            input: ToolInvocation,
        ) -> Result<PreOutcome<ToolInvocation>, HookError> {
            if input.call.name.ends_with("/write") {
                Ok(PreOutcome::Veto(Veto {
                    reason: "policy.no-writes".into(),
                    detail: None,
                    by: EntityId::new("no-writes"),
                }))
            } else {
                Ok(PreOutcome::Continue(input))
            }
        }
    }
    let rig = rig(64 * 1024).await;
    rig.dispatcher.register::<ToolExecute>(
        EntityId::new("no-writes"),
        Hook::Pre(Arc::new(NoWrites)),
        HookOrder::default(),
    );
    let admin = Caller::human("a", [Role::Admin]);
    let out = run(
        &rig,
        &admin,
        vec![
            call("c1", "write", json!({ "text": "x" })),
            call("c2", "count", json!({})),
        ],
    )
    .await;
    assert_eq!(
        result(&out[0]).error,
        Some(ToolError::Denied {
            tool: "demo/echo/write".into(),
            reason: "policy.no-writes".into()
        })
    );
    assert_eq!(
        result(&out[1]).render(),
        "0",
        "the server never ran the write"
    );
}

/// Abuse case 4: a denied tool call is not executed and the model receives a tool error.
#[tokio::test]
async fn abuse_denied_tool_call_is_not_executed_and_model_receives_a_tool_error() {
    let rig = rig(64 * 1024).await;
    rig.dispatcher.register::<ToolExecute>(
        EntityId::new("tools.policy"),
        Hook::Guard(Arc::new(ToolPolicy::default())),
        HookOrder::default(),
    );
    let watcher = Caller::human("w", [Role::Watcher]);
    let out = run(
        &rig,
        &watcher,
        vec![
            call("c1", "write", json!({ "text": "x" })),
            call("c2", "count", json!({})),
        ],
    )
    .await;
    assert!(
        matches!(result(&out[0]).error, Some(ToolError::Denied { .. })),
        "{:?}",
        out[0]
    );
    assert_eq!(result(&out[1]).render(), "0");
    let assignee = Caller::human("s", [Role::Assignee]);
    let out = run(
        &rig,
        &assignee,
        vec![
            call("c3", "write", json!({ "text": "x" })),
            call("c4", "count", json!({})),
        ],
    )
    .await;
    assert!(result(&out[0]).error.is_none());
    assert_eq!(result(&out[1]).render(), "1");
    // The server's descriptors carried the annotations into idempotency.
    let write = rig
        .cx
        .registry
        .latest::<dyn Tool>("demo/echo/write")
        .unwrap();
    assert!(!matches!(
        write.descriptor().idempotency,
        rusty_harness::harness::tools::Idempotency::Idempotent
    ));
    let _ = &rig.server;
}

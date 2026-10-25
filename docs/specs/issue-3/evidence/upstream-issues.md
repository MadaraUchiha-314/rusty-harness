# Upstream issues for the SDK gaps closed in `protocol_gap`

`crates/rusty-harness/src/app_service/a2a_server/protocol_gap/` closes two A2A 1.0
features the official Rust SDK (`a2a-server-lf` 0.5.3) does not implement (R19.9). Each
is to be filed against <https://github.com/a2aproject/a2a-rs> by the operator (the loop
has no GitHub identity for that repository); the drafts below are the issue bodies. When
the SDK ships a feature, the matching piece of `protocol_gap` is deleted.

## 1. `A2A-Extensions` is defined but not negotiated or echoed

- **Where:** `a2a_server::middleware::extract_service_params` copies every request header
  into `ServiceParams`; nothing intersects `A2A-Extensions` with the card's
  `capabilities.extensions`, nothing echoes the active set on the response, and the
  executor cannot tell which extensions a request activated.
- **Spec:** A2A 1.0, extensions: the server activates only extensions it advertises and
  echoes the activated set in the `A2A-Extensions` response header.
- **Our workaround:** `protocol_gap::ExtensionNegotiator` in an axum middleware writes the
  active set to a reserved header the executor reads and echoes it on the response.
- **Status:** draft, to be filed.

## 2. `ExecutorContext.user` is always `None`; `RequestAuthorizer` is synchronous

- **Where:** `DefaultRequestHandler::start_execution` builds `ExecutorContext { user: None, … }`
  and `RequestAuthorizer::authorize(&ServiceParams, Option<&str>)` is a plain `fn`, so an
  authorizer cannot consult a task store to decide whether the caller participates.
- **Spec:** A2A 1.0 security: credentials per the card's schemes, 401/403 before any
  operation, and task-scoped operations answered as `TaskNotFound` for a caller who may
  not see the task.
- **Our workaround:** authentication in the middleware, the caller's id in a reserved
  service parameter, and `a2a_server::GuardedHandler` wrapping the SDK handler with an
  async participant check and membership-filtered `ListTasks`.
- **Status:** draft, to be filed.

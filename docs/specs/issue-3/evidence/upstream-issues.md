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

## 3. `@a2ui/web_core` 0.14 basic catalog: unlabelled text field, primary button below AA

- **Where:** `universal/basic_catalog/components/TextField.js` renders the label
  (`<label>${props.label}</label>`) and the `<input class="a2ui-textfield">` as siblings
  with no `for`/`id` or `aria-labelledby`, so axe reports `label` (critical) on every
  text field;
  the default primary button colour on white text sits below 4.5:1, so axe reports
  `color-contrast` (serious) on `.a2ui-button-primary` (`web/tests/a11y/axe.spec.ts`,
  the A2UI form state).
- **Spec:** WCAG 2.1 AA 1.3.1 / 4.1.2 (name, role, value) and 1.4.3 (contrast).
- **Our workaround:** `web/src/components/app/a2ui-block.tsx` names each
  `input.a2ui-textfield` from its sibling label with `aria-label` (a `MutationObserver`
  keeps re-renders labelled); `web/src/styles.css` paints `.a2ui-button-primary` in the
  agent accent, which meets AA in both themes.
- **To file against:** <https://github.com/google/A2UI> (`web_core`, basic catalog).
- **Status:** draft, to be filed.

## 4. `@a2a-js/sdk` 1.3 needs Node's `Buffer` in the browser for raw parts

- **Where:** `Part.fromJSON`/`toJSON` (and so `SendMessageRequest.fromJSON` and every
  streamed event carrying a raw part) call `Buffer.from` for the `raw` field; a browser
  has no `Buffer`, so a page receiving an image artifact or sending an audio part throws
  `ReferenceError: Buffer is not defined` (`web/tests/visual/states.spec.ts`, the voice
  state, found it).
- **Spec:** A2A 1.0 ProtoJSON: `raw` is base64; `atob`/`btoa` or `Uint8Array.fromBase64`
  suffice in a browser.
- **Our workaround:** `web/src/polyfills.ts` installs the `buffer` package as
  `globalThis.Buffer` before the SDK loads (design.md D1: polyfill, no hand-rolled
  client).
- **To file against:** <https://github.com/a2aproject/a2a-js>.
- **Status:** draft, to be filed.

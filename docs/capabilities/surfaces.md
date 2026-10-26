# Capability: surfaces and renderers

> A surface is an A2A client that says what it can render; the harness sends it only
> that, with text for the rest. Agent-generated UI is A2UI 0.9.1 from the basic catalog,
> carried as a data part, typed in the core and rendered natively by the TUI.

## What it is

`interaction::surface::SurfaceCapabilities` is what a surface renders: media types, A2UI
catalogs, and whether it hosts MCP Apps. Over A2A a surface declares its catalogs in the
message metadata `a2uiClientCapabilities` (`{ "v0.9.1": { "supportedCatalogIds": […] } }`),
which the executor reads per request; `acceptedOutputModes` never reach the executor
through the SDK, so a surface that restricts media types runs the same negotiation on its
own side. `negotiate` keeps the parts the surface renders and replaces each other part
with a text part marked `alternativeFor: <media type>`: the part's own alt text, the A2UI
tree rendered as text, its filename or URL, or the catalog string
`surface.alternative.unrenderable`. A message that already carries the alternative (every
A2UI ask does) is not duplicated, and the A2UI extension is dropped from a message whose
A2UI parts all went.

`interaction::renderer::Renderer` is the native renderer entity: it owns a
`SurfaceCapabilities` and renders `RenderItem`s — the stream's `Message`, `Task`,
`StatusUpdate` and `ArtifactUpdate`, an `A2ui` payload lifted out of a part, and
`McpApp`, which exists so MCP Apps can arrive later without changing the interface;
nothing produces one and `supports` refuses it unless the `mcp_apps` flag is set.

`interaction::renderer::a2ui` is the A2UI 0.9.1 model: the four server-to-client
messages in their versioned envelope (`A2uiMessage`, written as `v0.9.1`, read as `v0.9`
or `v0.9.1`, any other version refused), the client's `action` and `error`
(`ClientMessage`), the eighteen basic-catalog components (`Component` with a
`ComponentKind` per entry, dynamic values as literal, `{ "path" }` binding or function
call), and the capability objects both sides exchange (`ClientCapabilities`,
`ServerCapabilities`). `part` builds the `DataPart` — the message list as `data`,
`application/a2ui+json` as the part's media type and as its `metadata.mimeType` — and
`parse_part` reads one back; `text_alternative` renders a payload's component tree as
lines of text.

## Current behaviour

- WHEN a request carries `a2uiClientCapabilities` naming the basic catalog THEN the
  stream it opens SHALL carry A2UI data parts; WHEN it does not THEN every A2UI part in
  that stream SHALL be replaced by its text alternative and the A2UI extension dropped
  from the message (R25.2, R25.4).
- WHEN the model calls `participant.ask` with `kind: a2ui` THEN the question SHALL be a
  list of A2UI messages from the basic catalog — anything else is refused as invalid
  input — and the status message SHALL carry the text alternative (the tool's `text`, or
  the form rendered as text) first and the typed data part second (R25.2, R25.4).
- WHEN the agent card is built THEN its A2UI extension entry SHALL advertise
  `supportedCatalogIds: [basic]` and `acceptsInlineCatalogs: false` (R25.4).
- WHEN a renderer is asked whether it supports an item THEN an MCP App SHALL need the
  `mcp_apps` flag and an A2UI payload its catalogs; everything else is the renderer's
  choice (R25.3, R25.5).
- WHEN a surface cannot show a part and nothing better is known THEN the stand-in text
  SHALL come from the string catalog in the server's locale (R25.6).

## Design

- [`design.md` § Surfaces and renderers][design] (R25).
- The specification: `specification/v0_9_1` in <https://github.com/google/A2UI>; two of
  its samples are fixtures under `crates/rusty-harness/tests/fixtures/a2ui/`.
- Code: `crates/rusty-harness/src/interaction/{surface,renderer}/`; the ask emission in
  `harness/channels/`; the per-request negotiation in `app_service/a2a_server/`.

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M5, task 33) | `SurfaceCapabilities` and negotiation, the `Renderer` trait, the A2UI 0.9.1 model, typed ask payloads, the card's catalog advertisement | [spec](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/), PR #11 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

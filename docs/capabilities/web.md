# Capability: the web renderer

> A browser chat on shadcn/ui and its AI Elements that talks to the harness through the
> official A2A TypeScript SDK and nothing else; one store per task reduces the stream,
> A2UI asks render through the official React renderer, and every word comes from the
> crate's string catalog.

## What it is

`web/` is a Vite + React 19 + Tailwind 4 package. The chrome is shadcn/ui and the AI
Elements, copied from their registries by `bun run --cwd web copy:elements` into
`src/components/{ui,ai-elements}` (the same files the shadcn CLI writes, fetched without
its prompts so the copy is reproducible): `Conversation` and `Message` for the
conversation, `Prompt Input` for the composer, `Tool` for tool calls, `Plan` and `Task`
for the task panel, `Model Selector` behind `/model`, with `Dialog` for the complete
confirmation (R26.1). The design tokens are the prototype's zinc neutrals, the orange
agent accent and the violet command tokens, light and dark through
`prefers-color-scheme` and `data-theme` (design.md § UI/UX design).

`src/a2a/client.ts` is the only module that reaches the harness: `@a2a-js/sdk`'s
`ClientFactory.createFromUrl`, `sendMessageStream`, `resubscribeTask` and `cancelTask`
(R26.2). Every message declares `a2uiClientCapabilities` for the basic catalog; the first
message of a conversation opens a long-running task and a message after a finished task
starts a new one with `referenceTaskIds` (R21.7). The command list and the model aliases
come from the agent card's `commands/v1` extension entry.

`src/store/task.ts` reduces the event stream into one `TaskView` per task — turns,
artifacts (chunks folded by id), the `task/v1`, `participants/v1` and `plan/v1`
payloads, and an activity timeline — keyed by attempt: a status update whose `task/v1`
metadata says `retracted: [n]` drops attempt `n`'s partial output (R26.6, R22.4).

`src/components/app/` composes the elements: the conversation (markdown through
`streamdown`, command tokens highlighted from the card's list, an A2UI ask as a form
whose actions go back as A2UI client messages on the ask's channel), the composer with
the `/` palette, the header with the state badge and the complete and cancel controls,
the task panel and the sidebar. `src/i18n.ts` reads the crate's `strings/en.json`
through `intl-messageformat` (R33.7, R25.6).

## Current behaviour

- WHEN the page loads THEN it SHALL connect to `?url=` or its own origin through the
  agent card and SHALL talk A2A only through the official SDK (R26.2).
- WHEN a task streams THEN its turns, artifacts, state, sub-state, plan and participants
  SHALL render as they arrive, and a retracted attempt's partial output SHALL disappear
  (R26.6).
- WHEN a status message carries an A2UI payload THEN it SHALL render through
  `@a2ui/react` with the basic catalog, and a button's action SHALL be sent as an A2UI
  client `action` on the ask's channel (R25.4).
- WHEN a message starts with a registered command THEN the token SHALL be highlighted
  in the conversation and the composer, and `/` SHALL open the palette (R26.5).
- WHEN the task is open THEN the header SHALL offer complete (confirmed first, naming the
  sub-tasks the policy cancels) and cancel; WHEN it has finished THEN both SHALL be
  disabled and the next message SHALL start a new task referencing it (R26.3, R21.7).
- WHEN text is shown to a person THEN it SHALL come from `strings/en.json`; the strings
  test fails on a key the renderer references that the catalog lacks (R33.7).
- WHEN the visual suite runs THEN every prototype state SHALL match its baseline still in
  light and dark, and the phone width SHALL scroll vertically only (R26.7).
- WHEN axe runs at WCAG 2.1 AA on a state THEN there SHALL be no violation in either
  theme, and `Enter`, `Shift+Enter`, `/` and `Esc` SHALL do what the hint says (NFR
  accessibility).
- WHEN the built bundles are scanned THEN no provider key pattern and no model provider
  host SHALL be found (abuse 11, boundary 7).

## Design

- [`design.md` § Web renderer][design] (R26) and the prototype
  [`design/web-renderer.html`](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design/web-renderer.html).
- Code: `web/src/`; checks: `bun run --cwd web test` (vitest + Testing Library, T1),
  `bun run --cwd web test:strings` (T14), `bunx tsc --noEmit -p web`,
  `bun run --cwd web build`; the conformance suite stays at `bun run --cwd web test:contract`
  (T3).
- The browser legs run the production bundle against a scripted A2A mock
  (`web/tests/support/mock-harness.ts`: the card and the JSON-RPC methods, one event
  stream per prototype state): `bun run --cwd web test:visual` keeps one still per state
  in light and dark plus the phone width under `tests/visual/__screenshots__` (T5; the
  baselines are captured in the Playwright image CI runs in, `test:visual:update`
  re-captures them), `bun run --cwd web test:a11y` runs axe at WCAG 2.1 AA on every state
  in both themes and walks the keyboard flow (T9), `bun run --cwd web test:bundle-scan`
  refuses a provider key pattern or a provider host in the built bundles (T8, abuse 11).
  `bun run --cwd web capture` records the multi-turn flow as the evidence GIF.
- Not yet on the wire, so not yet rendered from the stream: tool-call progress and
  streamed text deltas; the `Tool` composition (`src/components/app/tool-card.tsx`) is
  shown from the test-only fixtures page for the `tool` and `denied` stills. Voice
  arrives with task 39.

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M5, task 35) | The web renderer package: AI Elements copied in, the SDK client, the per-task store, A2UI through `@a2ui/react`, the composer with the palette, the task panel, strings from `en.json` | PR #11 |
| issue-3 (M5, task 36) | The visual contract on Playwright against a scripted mock, axe at AA in both themes, the keyboard flow, the bundle scan, the capture script | PR #11 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

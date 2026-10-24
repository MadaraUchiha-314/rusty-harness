---
type: testing-plan
phase: test-planning
workItem: issue-3
status: draft
approvedBy: []
overrides: {}
---

<!-- Written per the `the-loop:writing` skill: front-load each section's
     conclusion, draw it rather than describe it (3+ named parts -> a mermaid
     diagram), and keep the formal registers formal (EARS, abuse cases,
     RFC-2119, API contracts, schema descriptions). No length limit — length
     follows the change; the test is whether a sentence can come out without
     losing information. A gated section stays even when it is empty. -->

# Testing plan: rusty-harness, an opinionated production-grade agent harness

> Derived from the approved `requirements.md`/`bugfix.md` and `design.md`, **before**
> `tasks.md` — each task's `_Test:_` names a row of the matrix below. Authored at the
> `test-planning` node and **completed at the `verification` node**: the same file is
> written once as a plan and once as a record, so intent and outcome sit in one diff.
> See `reference/testing.md`.
>
> **This file is executable content.** It names commands an agent will run, so review it
> like code. Credentials appear **by reference only** (env var name, secret-store key),
> never by value.

**Fourteen rows apply, three do not; the proof of the epic is the demo (R32) run twice,
in `remote` and `none` durability, with the A2A server driven by the official TypeScript
SDK and every trust boundary of `design.md` holding under its negative test.** The
reader is the approver, who reviews this plan with the design at one gate. Everything
here runs from the repository's own commands
([local development](../../development/local-development.md), which CI mirrors); the only
external services are OpenAI and Temporal Cloud, reached by credential reference and
skipped, never failed, when absent (R31.4). Milestones (S3) run the rows that exist at
that point; the full matrix is the condition for the last milestone.

## Test matrix

> One row per candidate testing type. A type that does not apply is marked `n/a`
> **with a reason** — an unexplained blank is not a decision. Nothing here is mandatory
> in itself; the matrix is **work-item dependent**, and most work items use a handful of
> rows. Add types the catalogue does not list (chaos, load-soak, i18n, data-migration
> dry-run…) as extra rows.

| # | Type | Applies? | Scope / what it proves | Where it runs |
|---|------|----------|------------------------|---------------|
| T1 | Unit | yes | every module's public types through their public interface only (R31.1, R31.5): registry, hooks dispatcher order and policies, config layering, prompt templating and the ICU subset parser, plan DAG and cycle rejection, task state machine, context builder positions, compaction policy, command parser, plugin and skill validators, error mapping; web components (AI Elements composition, A2UI rendering, command highlighting) | `cargo nextest run --workspace --all-features`; `bun run --cwd web test` (vitest) |
| T2 | Integration (scenario) | yes | modules collaborating, Gherkin-documented (R31.1): the 19 scenarios in `design.md` §Testing strategy plus the 11 abuse scenarios, against a mocked `Llm` (recorded turns) and a mocked `Host`; the Temporal scenarios against the SDK's ephemeral dev server (`durable-embedded`) | `cargo nextest run --workspace --all-features -E 'kind(test)'` (`crates/rusty-harness/tests/`) |
| T3 | Contract | yes | the shape of every public interface (R3.6, R31.2): A2A protocol conformance driven by `@a2a-js/sdk` as the client (R31.3); the six `specs/a2a-ext/*.schema.json`; `rusty-harness.schema.json`; the Python stub (`stub_gen` diff) and the TypeScript declarations; the module tree vs `design.md`; prompt files vs registry; `strings/*.json` vs `en.json`; `cargo semver-checks` on the core crate at release | `cargo nextest run -p rusty_harness -E 'test(contract_)'`; `bun run --cwd web test:contract` (a vitest suite that boots the server binary and drives it with the official SDK); `cargo run -p rusty-harness-py --bin stub_gen && git diff --exit-code`; `cargo semver-checks` (release) |
| T4 | End-to-end | yes | the definition-of-done demo (R32) as one journey: TUI and web on one harness, A2UI from the basic catalog, a long-running root task over three turns with a channel ask, OpenAI GPT 6.1, run once in `remote` (Temporal Cloud) and once in `none`; skipped when `OPENAI_API_KEY`/`TEMPORAL_*` are absent | `cargo nextest run -p rusty_harness --features cli -E 'test(e2e_)' --run-ignored all` (tests are `#[ignore]`-gated on credentials); `bun run --cwd web test:e2e` (Playwright, browser leg) |
| T5 | UI / visual | yes | the built web renderer and TUI match the locked prototypes in `design/` state by state: screenshots of each verified state and a GIF of the multi-turn flow; TUI states captured from a pty | `bun run --cwd web test:visual` (Playwright screenshots vs `design/screenshots/`); `cargo nextest run -p rusty_harness --features tui -E 'test(tui_snapshot_)'` (ratatui `TestBackend` buffers) |
| T6 | Snapshot | yes | serialized outputs that must not drift: the agent card, the default `config.toml`, the generated JSON schemas, A2A extension payloads, the TUI frame buffers, rendered prompt fragments for a fixed `PromptArgs` | `cargo nextest run -p rusty_harness -E 'test(snapshot_)'` (`insta`) |
| T7 | Performance / load | yes | the NFR targets (Q13) as measured numbers, not gates: loop overhead per iteration (p50 < 5 ms, mocked model), TUI cold start (< 200 ms), CLI binary size (< 25 MB), wasm bundle (< 3 MB gzipped), prompt-cache hit rate on the demo flow (R15.2), inbox flood behaviour (abuse 8) | `cargo bench -p rusty_harness --bench loop_overhead` (criterion); `scripts/measure.sh` (sizes, cold start) run by the e2e leg |
| T8 | Security / abuse case | yes | one negative test per trust boundary and abuse case in `design.md` §Security design (11 scenarios, Gherkin-titled there), the credential canary across logs, traces, store, responses and card, the bundle scan for key patterns, `cargo deny` | `cargo nextest run --workspace --all-features -E 'test(abuse_)'`; `bun run --cwd web test:bundle-scan`; `cargo deny check` |
| T9 | Accessibility | yes | WCAG 2.1 AA on the web renderer (keyboard reachability, focus visibility, contrast in both themes) via axe; TUI `NO_COLOR` variant renders every state | `bun run --cwd web test:a11y` (Playwright + axe-core); `tui_snapshot_` with `NO_COLOR=1` |
| T10 | Migration / upgrade | n/a — the repository has no users, data or configuration before this work item; the config schema and the store layout are versioned from their first release so a later work item can add this row | | |
| T11 | Manual exploratory | yes | what automation cannot reach: voice on the TUI (push-to-talk, hold vs toggle) and on the web (microphone capture, playback), Realtime latency, the demo screen recording (R32.6); procedure in `evidence/manual.md` | the demo environment below, recorded |
| T12 | WASM / browser | yes | the core on `wasm32`: compile check, no tokio in the tree, a task end to end against a mocked `HttpClient` and host in Node and headless Chrome (R29.4–5), the published npm package's types | `cargo check --target wasm32-unknown-unknown -p rusty_harness --no-default-features --features rt-wasm`; `wasm-pack test --node` and `--headless --chrome` in `crates/rusty-harness-wasm`; `bun run test` there |
| T13 | Chaos / crash recovery | yes | R22.5's kill-the-worker tests, the store-journal crash test and the outbound-reconciliation crash test: kill the activity worker during `call_model` → one re-dispatch, no duplicate tool side effect; kill the workflow worker → replay on another worker with no repeated side effect; kill the process in `none` mode → restart resumes from the last persisted iteration (R14.4) | `cargo nextest run -p rusty_harness --features durable-embedded -E 'test(chaos_)'` (two worker processes against the ephemeral dev server; the test kills one with SIGKILL) |
| T14 | i18n / l10n | yes | R33: every prompt fragment resolves to a file and every fragment id in a trace maps back; `strings/en.json` keys compile to constants; a second-locale fixture (test-only, not shipped, Q19) falls back per key; placeholder sets match; the `no_raw_prompt_literals` lint | `cargo nextest run -p rusty_harness -E 'test(assets_)'`; `bun run --cwd web test:strings` |
| T15 | Python bindings | yes | awaitable and `*_sync` parity for every exported API, typed configuration round-trip, `pyright` strict on package and tests (R3.3, R30.2) | `uv run maturin develop && uv run pytest && uv run pyright` in `crates/rusty-harness-py` |
| T16 | Static checks | yes | the repository's lint floor on every PR: fmt, clippy pedantic with warnings denied, `missing_docs`, rustdoc, `tsc` strict, oxlint, markdownlint, commitlint, `cargo machete` | `cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo doc --no-deps --all-features`; `bun run lint && bun run format && bun run lint:md` |

## Scenarios & requirement trace

> Which requirement each row proves, and — for integration rows — the Gherkin
> `Scenario:` titles the tests will carry (`testing.gherkinDocstrings`). Do not paste the
> scenario table here: `the-loop scenarios --format markdown` renders it for the reviewer
> briefing.

| Row | Requirement(s) | Scenario / case |
|-----|----------------|-----------------|
| T1 | R1, R3, R4, R7, R10, R13, R14.1, R15, R16, R27.2, R30.4, R33 | unit cases per module, named after the public function under test |
| T2 | R1, R4, R5, R6, R8, R9, R10, R11, R12, R15, R16, R17, R18, R20, R21, R22, R23, R24, R25 | `Scenario: A registered default is replaced by a plugin without a code change` · `Scenario: A pre hook veto stops tool execution and the model is told` · `Scenario: A plugin with an invalid manifest leaves no entity behind` · `Scenario: A skill body is loaded on activation and not before` · `Scenario: Several tool calls run concurrently and return in issue order` · `Scenario: Switching /model applies to the next call without a restart` · `Scenario: A long-running task stays in input-required across three turns` · `Scenario: A follow-up to a completed task starts a new task with referenceTaskIds` · `Scenario: A participant ask round-trips through a channel` · `Scenario: Compaction keeps the pinned positions verbatim` · `Scenario: A loopback sub-agent result is indistinguishable from a remote one` · `Scenario: An inbox event for a running task is applied at the next loop boundary` · `Scenario: The heartbeat forwards an event whose signal failed` · `Scenario: Every operation of a task appears in one trace` · `Scenario: An A2UI ask renders in the web renderer and the TUI` · `Scenario: Remote hooks over JSON-RPC and MCP see the same typed context as local hooks` · `Scenario: A tool result above the size limit is truncated by the configured policy` · `Scenario: A sub-agent whose card fails validation is never shown to the model` · `Scenario: A task whose parent completes is canceled by the default policy` · `Scenario: A pre hook that rewrites a tool call after approval is re-checked by the guard` · `Scenario: A task watcher outside a channel sees the state change but not the question` · `Scenario: A duplicate inbox signal applies the event once` · `Scenario: A retried model call retracts the partial output of the failed attempt` · `Scenario: Completing a working task stops in-flight activities and ends completed` · `Scenario: A participant answering after the tool timeout resumes the original suspended call` · `Scenario: GetTask after a restart returns the full A2A task` · `Scenario: Spans across a signal and a retry link into one trace` |
| T3 | R2.3, R3.5–6, R19.1–3, R19.5, R30, R31.2–3, R33.5, R33.9 | `Scenario: The official TypeScript SDK drives every A2A 1.0 operation against the server` · `Scenario: A request with A2A-Version 0.3 is rejected` · `Scenario: An extension not advertised in the card is not activated` · `Scenario: The module tree matches design.md` · contract snapshots per schema and stub |
| T4 | R32.1–6, R21, R22.1, R25.4 | `Scenario: The demo runs end to end in remote mode` · `Scenario: The demo runs end to end in none mode` |
| T5 | R26.7, R25.4, R27, R28.2, NFR accessibility | one case per prototype state (web: streaming, tool, a2ui, idle, voice, command, unknown, denied, done; TUI: chat, a2ui, model, voice, done, first) · `Scenario: An image artifact renders in a graphics-capable terminal and degrades to alt text` |
| T6 | R19.2, R27.2, Q9, R19.3 | snapshot per serialized artifact |
| T7 | NFR performance, R15.2, abuse 8 | measured numbers recorded, compared with the targets |
| T8 | Security considerations (boundaries 1–7, abuse 1–11) | the 11 abuse scenarios titled in `design.md` §Security design (abuse 2 now includes the `ListTasks` membership filter), plus `Scenario: The canary credential appears nowhere observable` |
| T9 | NFR accessibility | axe has no violations at AA on every web state in light and dark; `NO_COLOR` TUI buffers are legible |
| T11 | R28.3–6, R27.5, R32.6 | manual procedures: push-to-talk hold and toggle, web voice, demo recording |
| T12 | R29.1, R29.4, R29.5, R8.1 (host MCP transport) | `Scenario: The wasm build runs a task against a mocked host` · `Scenario: An unavailable capability fails at construction on wasm32` |
| T13 | R22.3–5, R14.4, R17.4, R23.4 | `Scenario: Killing the activity worker re-dispatches the model call once` · `Scenario: Killing the workflow worker replays without a repeated tool side effect` · `Scenario: A restart in none mode resumes from the last persisted iteration` · `Scenario: A crash between two store writes leaves a consistent store on reopen` · `Scenario: Killing the worker after the peer accepted and before the id was recorded creates no duplicate remote task` · `Scenario: A crashed activity's failure and retry hooks run on the surviving worker` |
| T14 | R33.1–10, R7.4, R25.6, R27.8 | `Scenario: Every registered prompt fragment resolves to a markdown file` · `Scenario: A missing key in a locale falls back to en and a missing key in en fails the build` |
| T15 | R30.2, R3.3 | parity table generated from the stub, one test per exported name |
| T16 | R3.1, R3.4, NFR typing | the CI jobs themselves |

## Verification environment

> What the verification needs in order to run. the-loop **facilitates** verification; it
> does not own your environment — name the project's own commands here rather than
> expecting the loop to model the setup. Where an operator document already describes
> this, link the doc registered in `customInstructions.docs` instead of restating it.

- **Repositories:** this repository only (`web/` is a workspace member; sub-agents in
  the demo are a second instance of the same binary).
- **Services / containers:** none for T1–T3, T5–T9, T12, T14–T16 (mocked `Llm` and
  `Host`; the contract suite boots the server binary itself). T13 and the Temporal
  integration scenarios use the SDK's ephemeral dev server (`durable-embedded`
  downloads the `temporal` CLI once into `~/.cache/rusty-harness/temporal/`). T4 and T11
  use **Temporal Cloud** (`remote`) and **OpenAI**; the `none` leg of T4 needs OpenAI
  only.
- **Fixtures & data:** recorded model turns under `crates/rusty-harness/tests/fixtures/turns/`
  (JSON, scrubbed), a demo plugin under `tests/fixtures/plugins/demo/` (one skill, one
  stdio MCP server written as a tiny Rust test binary, one JSON-RPC hook), a
  second-locale fixture `tests/fixtures/strings/xx.json`, the locked prototypes under
  `design/` and their stills under `design/screenshots/` for T5.
- **Credentials:** **by reference only** — `OPENAI_API_KEY` (or keyring
  `rusty-harness/openai`), `TEMPORAL_ADDRESS`, `TEMPORAL_NAMESPACE`, `TEMPORAL_API_KEY`
  (or keyring `rusty-harness/temporal`). The canary run of T8 sets
  `OPENAI_API_KEY=canary-<random>` against a mocked provider. No Anthropic credential
  is needed (Out of scope).
- **Bring-up:** `bun install`, `cargo build --workspace --all-features`, then in
  `crates/rusty-harness-py`: `uv sync && uv run maturin develop`. Demo: the server with
  `cargo run --features cli -- --config tests/fixtures/demo/config.toml --no-tui`, the
  TUI with `cargo run --features cli -- --url http://127.0.0.1:7474`, the web renderer
  with `bun run --cwd web dev`.
  **Tear-down:** `Ctrl-C` the three processes; the ephemeral dev server shuts down with
  its test; `rm -rf ~/.cache/rusty-harness/demo-data`.
- **If bring-up fails:** record it under Verification results, leave the dependent
  activities unticked, and escalate — do not pass the gate on an environment that
  never came up.

## Evidence plan

> What will be captured, and where. Evidence is committed under
> `<specDir>/<id>/evidence/`; a link to a CI run that expires or to a local path is not
> evidence. **Textual evidence is markdown (`.md`), never `.txt`** — titled, one section
> per command, raw output in fenced blocks. Binary captures keep their own formats.
>
> **Redact before committing.** Captured output and screenshots routinely contain tokens,
> cookies, personal data and internal hostnames, and this directory is as public as the
> repository. If a capture cannot be redacted, do not commit it — say so in the results
> row instead. A secret that reaches a commit is rotated, not merely edited out.

| Row | Evidence | Path under `evidence/` |
|-----|----------|------------------------|
| T1, T15, T16 | test summaries (counts, duration), lint output | `unit.md`, `python.md`, `static.md` |
| T2 | `the-loop scenarios --format markdown` table + run output | `integration.md` |
| T3 | contract run output; the SDK-driven conformance log; the schema files themselves live under `specs/` | `contract.md` |
| T4 | the two demo runs: command, console output (scrubbed), the OTLP trace export (JSON, scrubbed), the Temporal workflow history (JSON, scrubbed of payload blobs), a screen recording of both renderers | `e2e-remote.md`, `e2e-none.md`, `e2e/trace-remote.json`, `e2e/history-remote.json`, `e2e/demo.webm` (or a link to a release asset if over the repository's size limit, stated in the row) |
| T5 | screenshots of each verified state; a GIF of the multi-turn flow | `ui/web-<state>.png`, `ui/tui-<state>.png`, `ui/multi-turn.gif` |
| T6 | the `insta` snapshot directory is the evidence; summary | `snapshot.md` |
| T7 | the criterion report summary and the size/cold-start table | `performance.md` |
| T8 | abuse-scenario run output; the canary search transcript; `cargo deny` output; bundle-scan output | `security-tests.md` |
| T9 | axe report per state; `NO_COLOR` buffers | `accessibility.md` |
| T11 | the written procedures with outcomes and timestamps | `manual.md` |
| T12 | wasm check, `cargo tree` proof, `wasm-pack test` output | `wasm.md` |
| T13 | the chaos run logs showing the kill, the re-dispatch and the replay | `chaos.md` |
| T14 | assets run output | `assets.md` |

## Verification activities

> The checklist the `verification` node gates on (`checkmarks: complete`). One line per
> thing that will actually be executed. Tick a line **only** when it has been run and its
> evidence recorded below. An activity that cannot be executed is **not** ticked: record
> why under Verification results and either replan this matrix (with the reason) or
> escalate.

- [ ] T16 — `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] T16 — `cargo doc --no-deps --all-features`, `cargo deny check`, `cargo machete`
- [ ] T16 — `bun run lint && bun run format && bun run lint:md && bunx tsc --noEmit -p web`
- [ ] T1 — `cargo nextest run --workspace --all-features -E 'kind(lib)'`
- [ ] T1 — `cargo test --doc --workspace --all-features`
- [ ] T1 — `bun run --cwd web test`
- [ ] T2 — `cargo nextest run --workspace --all-features -E 'kind(test)'` (includes the
  Temporal scenarios on the ephemeral dev server)
- [ ] T3 — `cargo nextest run -p rusty_harness -E 'test(contract_)'`
- [ ] T3 — `bun run --cwd web test:contract` (official TypeScript SDK as the A2A client)
- [ ] T3 — `cargo run -p rusty-harness-py --bin stub_gen` then `git diff --exit-code` on the stub
- [ ] T6 — `cargo nextest run -p rusty_harness -E 'test(snapshot_)'`
- [ ] T8 — `cargo nextest run --workspace --all-features -E 'test(abuse_)'` (includes the canary run)
- [ ] T8 — `bun run --cwd web test:bundle-scan`
- [ ] T12 — the wasm32 `cargo check` and the `cargo tree` tokio check from CI's `wasm-check` job
- [ ] T12 — in `crates/rusty-harness-wasm`: `wasm-pack test --node`, `wasm-pack test --headless --chrome`
- [ ] T12 — in `crates/rusty-harness-wasm`: `bun run build && bun run test`
- [ ] T13 — `cargo nextest run -p rusty_harness --features durable-embedded -E 'test(chaos_)'`
- [ ] T14 — `cargo nextest run -p rusty_harness -E 'test(assets_)'`
- [ ] T14 — `bun run --cwd web test:strings`
- [ ] T15 — in `crates/rusty-harness-py`: `uv run maturin develop && uv run pytest && uv run pyright`
- [ ] T5 — `bun run --cwd web test:visual`
- [ ] T5 — `cargo nextest run -p rusty_harness --features tui -E 'test(tui_snapshot_)'`
- [ ] T9 — `bun run --cwd web test:a11y` and the `NO_COLOR=1` TUI snapshot leg
- [ ] T7 — `cargo bench -p rusty_harness --bench loop_overhead` and `scripts/measure.sh`
- [ ] T4 — `cargo nextest run -p rusty_harness --features cli -E 'test(e2e_none_)' --run-ignored all`
  (needs `OPENAI_API_KEY`)
- [ ] T4 — `cargo nextest run -p rusty_harness --features cli -E 'test(e2e_remote_)' --run-ignored all`
  (needs `OPENAI_API_KEY`, `TEMPORAL_*`)
- [ ] T4 — `bun run --cwd web test:e2e` against the running demo
- [ ] T11 — the manual procedures in `evidence/manual.md` (voice on TUI and web; the demo recording)

## Verification results

> Authored empty at `test-planning` (as `_Not yet executed._`) and filled at
> `verification`. One row per executed activity: the exact command or procedure, the
> outcome, and a link to the committed evidence.

_Not yet executed._

| Activity | Command / procedure | Outcome | Evidence |
|----------|--------------------|---------|----------|
| | | | |

**Not executed:** _none yet._

## Review comments

> Appended by the-loop's `record-feedback` hook when a human gate approves with
> comments (issue-109). Append-only and attributed: an approval never silently
> discards a reviewer's suggestions, and the feedback travels with the document
> it concerns rather than living in a side-channel tracker.

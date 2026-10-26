---
type: evidence
workItem: "issue-3"
---

# Final validation: rusty-harness, an opinionated production-grade agent harness

> The `evidence` node's proof: what is presented to the user to show the acceptance
> criteria are met. **Summarised from the executed verification** — `testing-plan.md`'s
> Verification results — and mapped onto the acceptance criteria rather than
> re-derived. Raw captures (output, screenshots, recordings) sit beside this file under
> `evidence/`.

**In short:** every automated row of the test matrix ran on 2026-10-25 at `0a97f5b` and
passed, including both live demo runs (`none` on OpenAI, `remote` on Temporal Cloud) and
the browser leg. One gap is open: the manual voice procedures V1–V3 (R28 with a real
microphone and speakers) were not run; the operator will run them after the merge
([decided on #3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3#issuecomment-6409132309)).
Verification also found and fixed three defects: a plugin path that could not be read
was skipped silently, the chaos suite did not compile under its planned features, and
the web `test:strings` script had never run a test.

<!-- markdownlint-disable MD013 -->

## Final validation evidence

| Acceptance criterion | How it was proved | Where |
|----------------------|-------------------|-------|
| R1 Core entities and the registry | T1 registry unit tests; T2 *A registered default is replaced by a plugin without a code change* | [`unit.md`](unit.md), [`integration.md`](integration.md) |
| R2 Module structure mirrors the architecture | T3 `contract_module_tree_matches_design`: the crate's module tree is the one `design.md` draws | [`contract.md`](contract.md) |
| R3 Typing, documentation, interface review | T16 clippy pedantic with `missing_docs`, rustdoc `-D warnings`, `tsc` strict, `pyright` strict; T3 schema, stub and declaration contracts | [`static.md`](static.md), [`contract.md`](contract.md), [`python.md`](python.md) |
| R4 Hooks on the whole lifecycle | T1 dispatcher order and policies; T2 *A pre hook veto stops tool execution…*, remote hooks over JSON-RPC and MCP, re-check after rewrite | [`unit.md`](unit.md), [`integration.md`](integration.md) |
| R5 Agent plugins | T2 plugin scenarios, including *A configured plugin path that cannot be read fails discovery* (added in this run) | [`integration.md`](integration.md) |
| R6 Skills | T2 *A skill body is loaded on activation and not before* | [`integration.md`](integration.md) |
| R7 System prompt | T1 prompt templating; T6 rendered default prompts; T14 every fragment resolves to a file | [`unit.md`](unit.md), [`snapshot.md`](snapshot.md), [`assets.md`](assets.md) |
| R8 Tools and MCP | T2 concurrent tool calls in issue order, truncation policy, the stdio MCP fixture; T12 the host MCP transport | [`integration.md`](integration.md), [`wasm.md`](wasm.md) |
| R9 Models: LLM and System One | T2 *Switching /model applies to the next call without a restart*; T6 provider request snapshots; T4 live on `gpt-6.1-sol` | [`integration.md`](integration.md), [`snapshot.md`](snapshot.md), [`e2e-none.md`](e2e-none.md) |
| R10 Commands | T1 command parser; T2 *A person's command runs instead of the model*; T5 command and unknown-command states | [`unit.md`](unit.md), [`integration.md`](integration.md), [`ui.md`](ui.md) |
| R11 The core loop | T2 loop scenarios (inbox at the loop boundary, retraction on retry, completion stops in-flight activities); T7 loop overhead | [`integration.md`](integration.md), [`performance.md`](performance.md) |
| R12 Task | T2 task lifecycle scenarios; T6 extension payloads validated against their schemas | [`integration.md`](integration.md), [`snapshot.md`](snapshot.md) |
| R13 Plan and Step | T1 plan DAG and cycle rejection; T2 the plan tools and the plan artifact on the wire; T5 the task panel | [`unit.md`](unit.md), [`integration.md`](integration.md), [`ui.md`](ui.md) |
| R14 State and persistence | T13 *A restart in none mode resumes from the last persisted iteration*, *A crash between two store writes leaves a consistent store*; T2 *GetTask after a restart* | [`chaos.md`](chaos.md), [`integration.md`](integration.md) |
| R15 Context window manager | T1 context positions; T2 loopback sub-agent; T7 prompt-cache hit rate 97% on calls 2–4 | [`unit.md`](unit.md), [`integration.md`](integration.md), [`performance.md`](performance.md) |
| R16 Compaction | T2 *Compaction keeps the pinned positions verbatim* | [`integration.md`](integration.md) |
| R17 Sub-agents over A2A | T2 loopback and remote sub-agents, invalid card never shown; T13 no duplicate remote task after a crash | [`integration.md`](integration.md), [`chaos.md`](chaos.md) |
| R18 Participants and channels | T2 *A participant ask round-trips through a channel*, watcher redaction; T4 the A2UI ask in both renderers | [`integration.md`](integration.md), [`e2e-none.md`](e2e-none.md) |
| R19 A2A API server | T3 the official TypeScript SDK drives the server; version and extension negotiation; T6 the agent card | [`contract.md`](contract.md), [`snapshot.md`](snapshot.md) |
| R20 Inbox and events | T2 *An inbox event for a running task is applied at the next loop boundary*, duplicate signal applied once | [`integration.md`](integration.md) |
| R21 Task lifecycle and multi-turn | T2 three turns in `input-required`, follow-up with `referenceTaskIds`; T4 three turns on one open task | [`integration.md`](integration.md), [`e2e-none.md`](e2e-none.md) |
| R22 Durable execution | T13 kill the activity worker (one re-dispatch) and the workflow worker (no repeated side effect); T4 on Temporal Cloud, 85 history events | [`chaos.md`](chaos.md), [`e2e-remote.md`](e2e-remote.md) |
| R23 Heartbeat | T2 *The heartbeat forwards an event whose signal failed* | [`integration.md`](integration.md) |
| R24 Observability | T2 *Every operation of a task appears in one trace*, spans across a signal and a retry; T4 exported traces, one trace id per task | [`integration.md`](integration.md), [`e2e-none.md`](e2e-none.md) |
| R25 Surfaces and renderers | T2 *An A2UI ask renders in the web renderer and the TUI*; T4 both clients on one harness | [`integration.md`](integration.md), [`e2e-none.md`](e2e-none.md) |
| R26 Web renderer | T1 web components; T5 the prototype states in the CI image; T4 browser leg | [`unit.md`](unit.md), [`ui.md`](ui.md), [`e2e-none.md`](e2e-none.md) |
| R27 TUI renderer and CLI | T5 the TUI prototype states; T4 the TUI on the demo | [`ui.md`](ui.md), [`e2e-none.md`](e2e-none.md) |
| R28 Multi-modal input and output | T2 capture sent as a voice part, spoken reply queued; T5 voice states; T8 voice key never leaves the harness. **Not run:** T11 V1–V3 with a real microphone and speakers | [`integration.md`](integration.md), [`ui.md`](ui.md), [`security-tests.md`](security-tests.md), [`manual.md`](manual.md) |
| R29 WASM and browser | T12 wasm32 check without tokio, a task end to end in Node and headless Chrome, the npm package | [`wasm.md`](wasm.md) |
| R30 Packaging and SDK parity | T15 awaitable and `*_sync` parity; T3 stub and declarations; T7 the `dist` binary size | [`python.md`](python.md), [`contract.md`](contract.md), [`performance.md`](performance.md) |
| R31 Testing | the whole matrix: every row of Verification results ran except T11 V1–V3 | [testing-plan.md § Verification results](../testing-plan.md#verification-results) |
| R32 Definition-of-done demo | T4 `none` and `remote` live, the browser leg, the recording of both renderers (T11 V4), trace and workflow history | [`e2e-none.md`](e2e-none.md), [`e2e-remote.md`](e2e-remote.md), [`manual.md`](manual.md) |
| R33 Prompts and strings as assets | T14 fragments resolve, catalogs match `en`, locale fallback; T6 rendered prompts | [`assets.md`](assets.md), [`snapshot.md`](snapshot.md) |
| NFR performance | T7: p50 289 µs (5 ms), cold start 24 ms (200 ms), binary 19.9 MB (25 MB), wasm 1.29 MB (3 MB) | [`performance.md`](performance.md) |
| NFR accessibility | T9: axe at WCAG 2.1 AA in light and dark, the keyboard flow; `NO_COLOR` TUI states | [`accessibility.md`](accessibility.md) |
| NFR security | T8: 19 abuse scenarios and the credential canary, the bundle scan, `cargo deny` | [`security-tests.md`](security-tests.md), [`static.md`](static.md) |
| NFR documentation | 23 capability docs, one per module; `evidence/documentation.md` | [`documentation.md`](documentation.md) |

<!-- markdownlint-enable MD013 -->

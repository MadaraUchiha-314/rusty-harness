# Decisions

Index of durable decisions. Each record lives beside this file as
`decision-<nnn>.md`; add one line per record here.

| # | Decision | Design trade-off | Status |
|---|----------|------------------|--------|
| [001](decision-001.md) | Repository tooling baseline | — | accepted |
| [002](decision-002.md) | MSRV 1.92 for the Temporal Rust SDK | T7 | accepted |
| [003](decision-003.md) | The official A2A Rust SDK, protocol ≥ 1.0, one browser transport of our own | T1, T2 | accepted |
| [004](decision-004.md) | The workflow is started by the inbox; activities exchange store keys | T3, T4 | accepted |
| [005](decision-005.md) | Voice goes through the harness, as A2A audio parts | T6 | accepted |
| [006](decision-006.md) | The shipped binary is built size-first, and aborts on panic | Q8 | accepted |
| [007](decision-007.md) | OpenAI is reached through the Responses API; Chat Completions for compatibles | T8 | accepted |
| [008](decision-008.md) | Tools keep their dotted names; each provider request maps them | T8 | accepted |
| [009](decision-009.md) | The model selection belongs to the task | R9.5, R10 | accepted |

The design's other trade-offs (T5, T9–T17) were built as the design describes and need
no record of their own; the table in [`design.md` § Trade-offs & decisions][tradeoffs]
remains their index.

[tradeoffs]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md#trade-offs--decisions

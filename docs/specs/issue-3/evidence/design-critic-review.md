---
type: evidence
workItem: issue-3
---

# Design critic review: rusty-harness, an opinionated production-grade agent harness

> Written only when the work item selected the opt-in `design-critic-review` phase
> (issue-188) — a different model/harness reading the completed `design.md` against the
> requirements, before the testing plan and the task DAG are derived from it. The node
> blocks until this section is filled in.

## Design critic review

| Round | Critic (`<harness>/<model>`) | Outcome | Findings → disposition | Link |
|-------|-----------------------------|---------|------------------------|------|
| 1 | `codex/gpt-6.1-sol` (83 s; 67k input, 2.6k output tokens) | 15 new findings | 15 → will-fix, all applied to `design.md` in place, one commit each (`f87705a`…`7fc3497`): ListTasks membership filter; guard stage after mutable pre hooks; per-subscriber redaction of channel questions; atomic idempotent `commit`; inbox received/signalled/applied; outbound reconciliation by intent + `contextId`; attempt protocol for streamed output; dyn-compatible `Entity`; `MaybeSend` bounds everywhere; complete-from-working stop path; workflow-orchestrated retries with failure/retry hooks; suspending tools for participant asks; `TaskRecord` fold to the full A2A task; span-wrapping `in` hooks with persisted trace context; TUI images via `ratatui-image` | [findings on PR #6](https://github.com/MadaraUchiha-314/rusty-harness/pull/6#issuecomment-6387559531) (15 comments, each `[codex/gpt-6.1-sol]`-attributed with its reply) |

Two earlier invocations of the same critic did not count as rounds: the first ran in the
critic's default working directory (another checkout) and reported the files missing;
the second was cut by a 120-second default timeout. The round above ran with `--cwd`
set to this worktree and `--timeout 900`.

The operator's policy caps critic rounds at one (`criticReviewCount: 1`), so no second
round runs; the fifteen findings are resolved in place and `design-approval` reads the
revised design.

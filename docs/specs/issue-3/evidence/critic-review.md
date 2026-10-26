---
type: evidence
workItem: "issue-3"
---

# Critic review: rusty-harness, an opinionated production-grade agent harness

> The `critic-review` node's proof — a different model/harness reading the diff. Same
> procedure as the self-review round (`reference/reviewing.md`); a round that could not
> run is recorded as **`unavailable`** with the cause and does NOT count toward the
> operator's `reviews.criticReviewCount`.

Policy: `criticReviewCount: 1`. The configured critic is `codex`, model `gpt-6.1-sol`
(`the-loop critic list`).

<!-- markdownlint-disable MD013 -->

## Review cycles

| Round | Critic (`<harness>/<model>`) | Outcome | Findings → disposition | Link |
|-------|-----------------------------|---------|------------------------|------|
| 1 (attempts 1–2) | `codex/gpt-6.1-sol` via `the-loop critic run` | **unavailable** | none: both attempts printed `timed out` (exit 2, no envelope) at about 120 s, with `--timeout 900` and `--timeout 2400`, while a one-line prompt answered in 6 s. The verb cuts long rounds whatever `--timeout` says | — |
| 1 | `codex/gpt-6.1-sol`, run directly: `codex exec -m gpt-6.1-sol -s read-only` over the whole M7 diff, told the spec and the 27 self-review findings (94 s) | new findings | 3: 1 high, 2 medium. All fixed. CR1-4 (high), a fourth hole of the same class, was found while fixing CR1-1 and fixed too | [round 1](https://github.com/MadaraUchiha-314/rusty-harness/pull/13#issuecomment-6411141345), [CR1-4](https://github.com/MadaraUchiha-314/rusty-harness/pull/13#issuecomment-6411740211) |

### Round 1 findings

| Id | Severity | Finding | Disposition |
|----|----------|---------|-------------|
| CR1-1 | high | the SDK's `get_task`, `subscribe` and `cancel_task` skipped the participation check the server applies (abuse 2) | fixed [`2c33d6a`](https://github.com/MadaraUchiha-314/rusty-harness/commit/2c33d6a): `Caller::may_open`, one rule for the server's guard and the SDK |
| CR1-2 | medium | a cancel returned the channel's question unredacted, on the SDK and (beyond the report) on the server's `CancelTask` (R18.5) | fixed [`2c33d6a`](https://github.com/MadaraUchiha-314/rusty-harness/commit/2c33d6a), together with CR1-1 since they change the same lines |
| CR1-3 | medium | an SDK follow-up with a `taskId` and no `contextId` could not resume its task (R21.5) | fixed [`ad99887`](https://github.com/MadaraUchiha-314/rusty-harness/commit/ad99887) |
| CR1-4 | high | the SDK's `send_message` let a non-participant post into or complete a task (found while fixing CR1-1) | fixed [`12dd258`](https://github.com/MadaraUchiha-314/rusty-harness/commit/12dd258) |

The critic's fixes are new code that no review round has read. The security review that
follows checks the authorization they touch against the checklist.

<!-- markdownlint-enable MD013 -->

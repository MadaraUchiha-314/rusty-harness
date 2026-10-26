---
type: evidence
phase: needs-review
workItem: "issue-3"
---

# Self-review: rusty-harness, an opinionated production-grade agent harness

> The `self-review` node's proof. One row per round, per `reference/reviewing.md`:
> attribution prefix, own-comment marker, reply-first-then-fix, stop on zero new
> findings, escalate on a repeated finding. Outcome is one of: new findings ·
> zero (converged) · escalated · **unavailable** (the configured reviewer could not
> run — it does NOT count toward `reviews.selfReviewCount`, read with
> `the-loop critic policy`).

Policy (`the-loop critic policy`): `selfReviewCount: 2`, `criticReviewCount: 1`,
`stopOnNoNewFindings: true`, `escalateOnRepeatFinding: true`.

<!-- markdownlint-disable MD013 -->

## Review cycles

| Round | Reviewer | Outcome | Findings → disposition | Link |
|-------|----------|---------|------------------------|------|
| 1 | `[claude/opus-5.5]`, four parallel passes over the M7 diff (`loop/issue-3-m6-voice..83f5e11`): adapters, core loop, models/host/durable, release and packaging; every finding checked against the code before posting | new findings | 25: 4 high, 7 medium, 14 low. 24 fixed (one commit each, with a test where one can fail), 1 won't-fix (SR1-19) | [round 1](https://github.com/MadaraUchiha-314/rusty-harness/pull/13#issuecomment-6409258555), [SR1-25](https://github.com/MadaraUchiha-314/rusty-harness/pull/13#issuecomment-6409488238) |
| 2 | `[claude/opus-5.5]`, one pass over round 1's fixes (`83f5e11..aa082dc`), told round 1's findings so it adds rather than repeats | new findings | 2, both introduced by round-1 fixes: 1 high, 1 low. Both fixed. No round-1 finding came back, so nothing escalated on repetition | [round 2](https://github.com/MadaraUchiha-314/rusty-harness/pull/13#issuecomment-6410425910) |

### Round 1 findings

| Id | Severity | Finding | Disposition |
|----|----------|---------|-------------|
| SR1-1 | high | `/commands` from the TUI and web reached the model (server path skipped the check) | fixed [`574ad1b`](https://github.com/MadaraUchiha-314/rusty-harness/commit/574ad1b) |
| SR1-2 | high | two plan tool calls in one turn lost an update; shared temp file name | fixed [`7073195`](https://github.com/MadaraUchiha-314/rusty-harness/commit/7073195) |
| SR1-3 | high | npm platform binaries published without the executable bit | fixed [`f952471`](https://github.com/MadaraUchiha-314/rusty-harness/commit/f952471) |
| SR1-25 | high | TUI dropped plan and command payloads whose numbers crossed the wire as doubles (found fixing SR1-1) | fixed [`1c9c8cd`](https://github.com/MadaraUchiha-314/rusty-harness/commit/1c9c8cd) |
| SR1-4 | medium | SDK `get_task` skipped channel redaction | fixed [`364b23b`](https://github.com/MadaraUchiha-314/rusty-harness/commit/364b23b) |
| SR1-5 | medium | a redirect took a request outside `allowOrigins` | fixed [`b1b2df5`](https://github.com/MadaraUchiha-314/rusty-harness/commit/b1b2df5) |
| SR1-6 | medium | the wasm SDK needed `unsafe-eval` (strict CSP broke it) | fixed [`5e1f507`](https://github.com/MadaraUchiha-314/rusty-harness/commit/5e1f507) |
| SR1-7 | medium | a stream cut before its stop counted as a finished turn (all three decoders) | fixed [`3d08170`](https://github.com/MadaraUchiha-314/rusty-harness/commit/3d08170) |
| SR1-8 | medium | `http://` Temporal address sent the API key in clear text | fixed [`909b473`](https://github.com/MadaraUchiha-314/rusty-harness/commit/909b473) |
| SR1-9 | medium | first npm release could not use trusted publishing | fixed (docs) [`1c71337`](https://github.com/MadaraUchiha-314/rusty-harness/commit/1c71337) |
| SR1-10 | medium | Python `Config` skipped the environment layer (R30.4) | fixed [`fe648fe`](https://github.com/MadaraUchiha-314/rusty-harness/commit/fe648fe) |
| SR1-11 | low | function call without `call_id` became `""` | fixed [`f6f65b8`](https://github.com/MadaraUchiha-314/rusty-harness/commit/f6f65b8) |
| SR1-12 | low | unparsable tool arguments became `{}` | fixed [`6a3d886`](https://github.com/MadaraUchiha-314/rusty-harness/commit/6a3d886) |
| SR1-13 | low | stream failures classified without the provider's code | fixed [`3819a68`](https://github.com/MadaraUchiha-314/rusty-harness/commit/3819a68) |
| SR1-14 | low | history-only tool name could alias a current tool | fixed [`82e8c3e`](https://github.com/MadaraUchiha-314/rusty-harness/commit/82e8c3e) |
| SR1-15 | low | `keyring:service=a/b` parsed as service/user | fixed [`fbbddc7`](https://github.com/MadaraUchiha-314/rusty-harness/commit/fbbddc7) |
| SR1-16 | low | `file:~/` with HOME unset read `./~/…` | fixed [`3078447`](https://github.com/MadaraUchiha-314/rusty-harness/commit/3078447) |
| SR1-17 | low | partial npm publish could not be re-run | fixed [`477fee9`](https://github.com/MadaraUchiha-314/rusty-harness/commit/477fee9) |
| SR1-18 | low | musl got a misleading reinstall message | fixed [`813cf1d`](https://github.com/MadaraUchiha-314/rusty-harness/commit/813cf1d) |
| SR1-19 | low | `[cli]` extra does not pin `rusty-harness-cli` | **won't-fix**: no coupling between the two, and a static pin breaks release-plz releases ([reply](https://github.com/MadaraUchiha-314/rusty-harness/pull/13#issuecomment-6410015489)) |
| SR1-20 | low | dropped wasm fetches and sockets were never aborted | fixed [`050bcfe`](https://github.com/MadaraUchiha-314/rusty-harness/commit/050bcfe) |
| SR1-21 | low | `A2AError.code` missing from the `.pyi` | fixed [`828ab03`](https://github.com/MadaraUchiha-314/rusty-harness/commit/828ab03) |
| SR1-22 | low | `plan.step_done` reported a veto as a failure | fixed [`15771e1`](https://github.com/MadaraUchiha-314/rusty-harness/commit/15771e1) |
| SR1-23 | low | plan hooks got an empty registry | fixed [`4f8f977`](https://github.com/MadaraUchiha-314/rusty-harness/commit/4f8f977) |
| SR1-24 | low | dropping a Python `Server` stops it, undocumented | fixed (docs) [`ab9e584`](https://github.com/MadaraUchiha-314/rusty-harness/commit/ab9e584) |

### Round 2 findings

| Id | Severity | Finding | Disposition |
|----|----------|---------|-------------|
| SR2-1 | high | server-path commands ran as a forged human Admin (introduced by SR1-1): a watcher or a peer agent could switch the process-wide model | fixed [`6bb5670`](https://github.com/MadaraUchiha-314/rusty-harness/commit/6bb5670) |
| SR2-2 | low | `PlanLocks` left table entries when a call was cancelled (introduced by SR1-2) | fixed [`06e7b48`](https://github.com/MadaraUchiha-314/rusty-harness/commit/06e7b48) |

The policy's two rounds are used. Round 2 was not a zero-finding round, so this record
says plainly that the rounds ran out rather than converged. Round 2's fixes are new code
that no self-review round has read; the critic round reads the whole diff, them included.

Observations outside the round's scope, not changed:

- `cargo clippy -p rusty_harness --features durable-temporal` without `durable-embedded`
  flags `unused_async` on the embedded-mode stubs.
- `cargo clippy --target wasm32-unknown-unknown` flags about 20 lints in the core crate
  (`arc_with_non_send_sync` and others).

CI runs neither configuration (it lints `--all-features` natively and only checks
wasm32), so both predate this work. Each is a candidate for a follow-up that adds the
configuration to CI.

One CI failure came from a round-1 fix: `WASM + TypeScript` on `5e1f507`, an import
left unused by SR1-6. It was healed in `aa082dc`.

<!-- markdownlint-enable MD013 -->

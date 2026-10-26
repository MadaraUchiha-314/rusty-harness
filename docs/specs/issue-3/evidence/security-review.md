---
type: evidence
workItem: "issue-3"
---

# Security review: rusty-harness, an opinionated production-grade agent harness

> The `security-review` node's proof. Required before ready-to-ship unless an authorized
> human declared the phase away at `phase-selection`. See `reference/security.md`.

<!-- markdownlint-disable MD013 -->

## Security review (gate)

- **Mechanism:** the-loop checklist (`reference/security.md`), verified against the M7
  diff (`loop/issue-3-m6-voice..HEAD`). No built-in security-review skill is available
  in this session. The round is posted on the PR:
  [security review](https://github.com/MadaraUchiha-314/rusty-harness/pull/13#issuecomment-6412373686).
- **Outcome:** one finding, fixed. Items 1–7 pass with evidence. Item 8 failed in part
  (SEC-1) and passes after the fix.
- **Findings:**

  | Id | Severity | Finding | Disposition |
  |----|----------|---------|-------------|
  | SEC-1 | low | `web-time`, `jsonschema` (dev) and `criterion` (dev), added in M7, were not justified in `design.md`'s dependency table | fixed [`2982ba4`](https://github.com/MadaraUchiha-314/rusty-harness/commit/2982ba4); `cargo deny check` passes all four checks |
  | SEC-2 | low | RUSTSEC-2026-0391, -0398 and -0403 (published after the review) against zstd 0.13 / zstd-safe 7, reached only through the Temporal SDK's dev server (`zip` 8, pinned upstream); no fixed release is reachable | accepted with reasons in `deny.toml`: `zip` reads through the read-side decoder with a fresh context, and `seekable` is off, so no affected path is used; revisit when the SDK moves to `zip` 9 |

- **Human sign-off:** approved by MadaraUchiha-314 on 2026-10-26
  ([comment](https://github.com/MadaraUchiha-314/rusty-harness/issues/3#issuecomment-6421744302)),
  in reply to the [request](https://github.com/MadaraUchiha-314/rusty-harness/issues/3#issuecomment-6421685956)
  that listed the remaining risks. The risk tier is 4 (`requirements.md`). Accepted: voice on
  a real microphone stays unverified until V1–V3 run after the merge, and SEC-2. The
  process-wide `/model` risk was removed before the sign-off ([`0ff3fd8`](https://github.com/MadaraUchiha-314/rusty-harness/commit/0ff3fd8)).

### The checklist, item by item

| # | Check | Result | Evidence |
|---|-------|--------|----------|
| 1 | Trust boundaries enforced where § Security design says | pass | M7's SDK and wasm surfaces are brought under boundaries 2 and 10 by the reviews: participants only (CR1-1, CR1-4), redaction (SR1-4, CR1-2), command roles from the task (SR2-1). Boundary 11 holds, redirects included (SR1-5). 19 of 19 `abuse_` tests pass at HEAD. |
| 2 | Untrusted input constrained at ingress | pass | Configuration through the schema and `SecretRef` (literals refused; SR1-15). An unreadable plugin path fails the build. Strict model-output decoding (SR1-7, 11, 12). Wire-name mapping (SR1-14). |
| 3 | Untrusted content cannot steer privileged behaviour | pass | Commands only from human participants, with their task roles (SR2-1). Data-role framing and tool policy are unchanged (abuse tests). |
| 4 | No secrets in code, config, logs or fixtures | pass | A scan of the M7 diff found no key patterns, no Temporal address or namespace, and no local paths. Demo evidence is scrubbed. Canary and bundle scan pass. |
| 5 | AuthZ fails closed | pass | A store error gives no commander. `may_open` refuses an outsider. A Temporal key over http to a remote host is refused (SR1-8). A redirect with an allowlist errors (SR1-5). `~` without `HOME` is refused (SR1-16). |
| 6 | Least privilege | pass | Release jobs have per-job permissions and publish by OIDC with no stored tokens. CI never uses `pull_request_target`. The wasm host gets only the capabilities the page passes. |
| 7 | Every abuse case has a passing negative test | pass | [`security-tests.md`](security-tests.md), plus the reviews' new negative tests (a watcher's `/model`; an outsider's read, follow, post and cancel; a cancel's redaction; the Temporal key over http; a redirect; the CSP leg). |
| 8 | New dependencies justified and from trusted sources | pass after SEC-1 | `cargo deny check`: advisories, bans, licenses and sources are ok; the design's table now lists all three. |

<!-- markdownlint-enable MD013 -->

---
type: evidence
workItem: "issue-3"
---

# Pull requests: rusty-harness, an opinionated production-grade agent harness

Every pull request opened for this work item, in the order opened. The spec chain is
iterated on the first one (the surface chosen at phase selection); implementation
pull requests are added as milestones ship.

## Pull requests

| PR | Repository | Branch | Carries | Opened | State |
|---|---|---|---|---|---|
| [#6](https://github.com/MadaraUchiha-314/rusty-harness/pull/6) | MadaraUchiha-314/rusty-harness | `loop/issue-3-spec` | the spec chain: `requirements.md` (later `design.md`, `testing-plan.md`, `tasks.md`) | 2026-10-24 | open |
| [#7](https://github.com/MadaraUchiha-314/rusty-harness/pull/7) | MadaraUchiha-314/rusty-harness | `loop/issue-3-m1-foundations` (base `loop/issue-3-spec`) | M1 foundations: tasks 1–10 (stack position 1 of 7) | 2026-10-24 | open (ready at C1) |
| [#8](https://github.com/MadaraUchiha-314/rusty-harness/pull/8) | MadaraUchiha-314/rusty-harness | `loop/issue-3-m2-harness` (base `loop/issue-3-m1-foundations`) | M2 harness core: tasks 11–19 (stack position 2 of 7) | 2026-10-24 | open (ready at C2) |
| [#9](https://github.com/MadaraUchiha-314/rusty-harness/pull/9) | MadaraUchiha-314/rusty-harness | `loop/issue-3-m3-service` (base `loop/issue-3-m2-harness`) | M3 app service layer: tasks 20–27 (stack position 3 of 7) | 2026-10-24 | open (ready at C3) |
| [#10](https://github.com/MadaraUchiha-314/rusty-harness/pull/10) | MadaraUchiha-314/rusty-harness | `loop/issue-3-m4-durable` (base `loop/issue-3-m3-service`) | M4 durable execution: tasks 28–32 (stack position 4 of 7) | 2026-10-25 | open (ready at C4) |
| [#11](https://github.com/MadaraUchiha-314/rusty-harness/pull/11) | MadaraUchiha-314/rusty-harness | `loop/issue-3-m5-renderers` (base `loop/issue-3-m4-durable`) | M5 surfaces: tasks 33–37 (stack position 5 of 7) | 2026-10-25 | open (ready at C5) |
| [#12](https://github.com/MadaraUchiha-314/rusty-harness/pull/12) | MadaraUchiha-314/rusty-harness | `loop/issue-3-m6-voice` (base `loop/issue-3-m5-renderers`) | M6 multi-modal and voice: tasks 38–41 (stack position 6 of 7) | 2026-10-25 | open (ready at C6) |
| [#13](https://github.com/MadaraUchiha-314/rusty-harness/pull/13) | MadaraUchiha-314/rusty-harness | `loop/issue-3-m7-ship` (base `loop/issue-3-m6-voice`) | M7 packaging, parity, docs and the demo: tasks 42–52 (stack position 7 of 7) | 2026-10-25 | open, ready for review since 2026-10-25 (the [reviewer briefing](https://github.com/MadaraUchiha-314/rusty-harness/pull/13#issuecomment-6409174862) posted) |

Spec chain artifact (rendering of the files above, not a source of truth):
<https://claude.ai/artifact/5mQ1GX58ddgNNrvmzXYWxw>

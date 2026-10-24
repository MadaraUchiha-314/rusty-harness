---
type: evidence
workItem: issue-3
---

# Spec explainer: rusty-harness, an opinionated production-grade agent harness

> Written by the `spec-explainer` node (issue-488): where the spec explainer and the
> spec quiz were published, so the `spec-quiz` gate can link the author to them. The
> node blocks until this section names a `Location`. How to write the page itself:
> the skill's `reference/explainer.md`.

## Spec explainer

| Field | Value |
|---|---|
| Depth | deep |
| Location | <https://claude.ai/artifact/5mQ1GX58ddgNNrvmzXYWxw> (tabs "Spec explainer" and "Spec quiz") |
| Published as | spec-chain artifact tab |
| Questions | 5 |

Depth is `deep` because the work item introduces a new subsystem (the whole harness),
changes public contracts in three languages and sits at risk tier 4. The page covers the
approved `requirements.md`, `design.md` and `testing-plan.md`; the five questions test
the task lifecycle, where the durable workflow starts, the guard stage, sub-states in
extension metadata, and activity replay semantics. Correct-answer positions are 3, 1, 4,
2, 3.

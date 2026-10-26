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

<!-- markdownlint-disable MD034 -- the gate parses a bare URL from the Location row -->
| Field | Value |
|---|---|
| Depth | deep |
| Location | https://claude.ai/artifact/5mQ1GX58ddgNNrvmzXYWxw |
| Published as | spec-chain artifact tab |
| Questions | 5 |
<!-- markdownlint-enable MD034 -->

The page is the artifact's "Spec explainer" and "Spec quiz" tabs.

Depth is `deep` because the work item introduces a new subsystem (the whole harness),
changes public contracts in three languages and sits at risk tier 4. The page covers the
approved `requirements.md`, `design.md` and `testing-plan.md`; the five questions test
the task lifecycle, where the durable workflow starts, the guard stage, sub-states in
extension metadata, and activity replay semantics. Correct-answer positions are 3, 1, 4,
2, 3.

## Rebuilt after the spec changed

The spec changed after the page was first built. `tasks.md` gained tasks 53 and 54 from
the task 47 sweep. `testing-plan.md` gained its Verification results, the T13 command
naming `server`, and T11 replanned to the demo recording, with V1–V3 deferred until after
the merge. The page was rebuilt from the current files and republished to the same URL
(version 16, 2026-10-26). Its "Spec explainer" and "Spec quiz" tabs were re-checked
against those changes: neither refers to the parts that changed, so both stand as
written, and the five questions and their answers are unchanged.

---
type: evidence
workItem: issue-3
---

# Explainer: rusty-harness, an opinionated production-grade agent harness

> Written by the `code-explainer` node (issue-488): where the explainer of the change and
> the code quiz were published, so the `code-quiz` gate can link the author to them. The
> node blocks until this section names a `Location`. How to write the page itself: the
> skill's `reference/explainer.md`.

## Explainer

<!-- markdownlint-disable MD034 -- the gate parses a bare URL from the Location row -->
| Field | Value |
|---|---|
| Depth | deep |
| Location | https://claude.ai/artifact/5mQ1GX58ddgNNrvmzXYWxw |
| Published as | spec-chain artifact tab |
| Questions | 5 |
<!-- markdownlint-enable MD034 -->

The page is the artifact's "Explainer" and "Code quiz" tabs, beside the spec explainer
and spec quiz on the same URL (version 18).

Depth is `deep` because milestone 7 changes the reader's model of the system, not just
its code:

- two new language surfaces over one SDK;
- distribution and release;
- two runtime behaviours the spec promised (plan tools, commands);
- the authorization rules the reviews tightened (one participation rule at every door,
  command callers rebuilt from the task's participant record).

The walkthrough follows a message through the system:

- the SDK and server doors;
- a command;
- the plan;
- the model call;
- the adapters;
- shipping.

It ends with risks and limits. The five questions mix code-level and spec-level ones:

- a watcher's `/model` on the server;
- a follow-up naming only its task;
- two plan steps recorded in one turn;
- a cut model stream;
- why the browser host's `Send`/`Sync` wrapper is sound.

Correct-answer positions are 2, 4, 1, 3 and 2, counting from 1.

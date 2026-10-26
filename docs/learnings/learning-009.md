# Learning 009: the-loop's tooling has defaults and parsers that must be met exactly

- **Date:** 2026-10-25
- **Source:** system-feedback
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3) (spec chain and M1–M7)
- **Category:** process

## What happened

Three separate stalls came from the harness's own tooling, not from the code:

- **The critic ran in the wrong place.** Without `--cwd`, `the-loop critic run codex`
  ran in its configured default checkout, where the spec files do not exist. Without
  `--timeout`, a real review was killed at the 120-second default, though the CLI
  config names 900 s.
- **The quiz box was never posted.** The spec-explainer record wrote the artifact URL in
  angle brackets. The gate reads a `Location` starting with `<` as the template's
  unfilled placeholder, so it never posted the quiz checkbox.
- **Commits were rejected for case.** CI's commitlint (`config-conventional`) rejects a
  subject whose first character is uppercase, so `docs: R33 …` failed, as did any
  header over 100 characters.

## Learning

- Invoke the critic as `the-loop critic run codex --root "$PWD" --cwd "$PWD" --timeout
  900 …`, in the background.
- Write an evidence record's `Location` as a bare URL, with a markdownlint exception
  for MD034.
- Start every commit subject with a lowercase verb, put ids after it, and stay within
  100 characters. Run `git log --format=%B -n 1 HEAD | bun run --silent commitlint`
  before pushing.

## Applied in

`docs/specs/issue-3/evidence/spec-explainer.md` (commit `d689cd7`), every critic round
under `docs/specs/issue-3/evidence/`, the commit history of PRs #6–#13.

# Learning 007: git converts shipped text assets to CRLF on Windows

- **Date:** 2026-10-25
- **Source:** system-feedback (CI, `Wheels / Wheel (windows-x86_64)` on PR #13)
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3) (M7, task 43)
- **Category:** platform

## What happened

The prompt files ship inside the binary through `include_str!`. On a Windows checkout,
git's line-ending conversion turned them into CRLF, and the front-matter parser looked
for `---\n`. `PromptRegistry::builtin` then panicked with "missing front matter", so
no harness could be built on Windows at all. The skills parser already normalised CRLF.
The prompt parser did not, and no Windows job had built a harness until the Python
wheel's tests did.

## Learning

Any parser of text embedded at compile time or read from a person's disk normalises
line endings before matching structure. A test with CRLF input sits beside each such
parser, so the rule does not depend on which CI leg happens to run on Windows.

## Applied in

`assets` prompt front matter (`a_fragment_with_crlf_line_endings_parses_the_same`),
[capability: assets](../capabilities/assets.md).

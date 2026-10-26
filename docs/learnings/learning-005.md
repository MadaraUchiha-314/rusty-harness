# Learning 005: a live provider run finds what a mocked model cannot

- **Date:** 2026-10-25
- **Source:** system-feedback
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3) (M7, tasks 45–46)
- **Category:** testing

## What happened

Every loop, tool and context test ran against a scripted model, and all of them passed.
The first live runs against OpenAI (the T7 cache measurement, then the T6 demo) failed
or misbehaved on their first request:

- dotted tool names were refused with a 400 ([decision-008](../decisions/decision-008.md));
- Chat Completions refused function tools with reasoning ([decision-007](../decisions/decision-007.md));
- a `file:` system prompt was never loaded, so the model saw the default;
- the Temporal client connected to Cloud without TLS;
- `~` in a store path was taken literally.

A mock accepts whatever the client sends, so none of these could fail there.

## Learning

A mocked model proves the loop. Only the real endpoint proves the wire. Run the
cheapest live call (one turn, built-in tools attached) as soon as a provider client
exists, not at the demo milestone. When a live run fails, add a mocked test that pins
the provider's rule (a name pattern, a refused field), so the rule stays enforced in CI
without credentials.

## Applied in

`models/wire_names.rs` and its tests, `models/openai/responses.rs`, the
`e2e_none_*` and `e2e_remote_*` runs in `crates/rusty-harness/tests/e2e.rs` (skipped
without credentials), [capability: models](../capabilities/models.md).

# Decision 007: OpenAI is reached through the Responses API; Chat Completions for compatibles

- **Status:** accepted
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3)
- **Design:** [`docs/specs/issue-3/design.md`][design] § Models (R9), trade-off
  T8 (own thin LLM clients); found by testing plan T7 (the cache measurement)

## Decision

`OpenAiLlm` speaks two wires. Without configuration, OpenAI's own endpoint gets the
**Responses API** (`/v1/responses`) and a model with a `base_url` gets **Chat
Completions**, which is what OpenAI-compatible servers implement. One key overrides it:

```toml
[models.fast]
provider = "openai"
model = "gpt-6.1-sol"
api = "responses" # or "chat-completions"
```

The Responses body sends `store: false`, so OpenAI keeps no conversation state; the
harness stays the only owner of the history (R21) and every request is self-contained.

## Why

The design assumed Chat Completions for OpenAI. The first live run against
`gpt-6.1-sol` was refused: current OpenAI models reject function tools combined with
reasoning on Chat Completions ("use /v1/responses or set reasoning_effort to 'none'").
Every harness turn carries tools (`agent.call`, `participant.ask`, the plan tools), so
turning reasoning off was the only alternative, and it would cost answer quality on the
provider the demo uses. Dropping Chat Completions instead would cut off every
OpenAI-compatible server (vLLM, Ollama, LiteLLM), which implement only that wire.

## Consequences

- Two request builders and two stream decoders in `models/openai/`, each with a request
  snapshot and a decoder test. Both decode cached-token usage, which the cache-hit
  measurement (T7) reads.
- The configuration schema, the TypeScript `Config` and the Python stub carry `api`.
- The defaults depend on whether `base_url` is set. [The models capability doc](../capabilities/models.md)
  states this rule.

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

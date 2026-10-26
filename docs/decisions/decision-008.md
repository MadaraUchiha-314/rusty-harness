# Decision 008: tools keep their dotted names; each provider request maps them

- **Status:** accepted
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3)
- **Design:** [`docs/specs/issue-3/design.md`][design] § Tools and MCP (R8), § Models (R9);
  found by testing plan T7

## Decision

Tool names inside the harness stay namespaced with dots (`agent.call`,
`participant.ask`, `plan.update`, `<mcp-server>.<tool>`): the registry, hooks,
permissions, traces and plugins all use them. At the provider boundary,
`WireNames` is built for each request from that request's tools. It:

- rewrites each name to `^[a-zA-Z0-9_-]{1,64}$` (other characters become `_`, a
  numeric suffix on collision, 64 characters at most);
- applies the same rule to tool calls already in the history;
- maps the model's tool calls back to the harness names before dispatch.

## Why

OpenAI and Anthropic both refuse a dotted tool name with a 400, so every real request
carrying the built-in tools failed. Renaming the tools to underscores would have
changed a name that hook matchers, permission rules and MCP namespacing already depend
on, and it would still be wrong for an MCP server whose tool names carry other
characters. Mapping at the wire keeps one naming rule inside the harness and confines
provider rules to the provider clients.

## Consequences

- A hook, permission or trace never sees a wire name. A model's reasoning text may
  mention one, which is cosmetic.
- The mapping is per request and deterministic, so a cached prompt prefix stays
  identical from call to call (R15.2).

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

# Capability: entities and the registry

> One identity and one trait per pluggable unit, and one place to resolve or replace it.
> The organized view of the specs that shaped it; the raw specs stay the historical record.

## What it is

Everything the harness can swap — a prompt, skill, tool, model, hook, plugin, command,
agent, channel, surface, renderer, store or durable backend — is an **entity**: it
implements the trait of its kind and carries an `EntityId` of `(id, version)`. The
`Registry` holds one typed map per trait object, so `registry.get::<dyn Tool>(&id)`
returns the same instance whether it runs in process or is a proxy to something remote.
Used by the SDK (`HarnessBuilder`), by plugins, and by every module that resolves a
collaborator by name.

## Current behaviour

- The system SHALL identify every registered entity by `EntityId { id, version }`;
  `version` is `None` for unversioned kinds and for registrations that state none.
- WHEN an identity is already registered for a trait THEN `register` SHALL fail with
  `RegistryError::Conflict` naming the kind, the identity and the holder's provenance;
  `replace` SHALL swap the instance and return the previous one.
- WHEN a lookup finds nothing THEN the system SHALL fail with `RegistryError::NotFound`
  naming the kind and the identity, never falling back to another entity.
- WHEN an entity is remote THEN its `placement()` SHALL name the protocol and address
  (`A2a`, `Mcp`, `Http`) and callers SHALL NOT need to know.
- `latest(id)` SHALL return the highest registered version of an id.

## Design

- [`design.md` § Registry and entities][design] (R1, R5.6).
- Code: `crates/rusty-harness/src/registry/`.

## History

| Work item | What changed | Links |
|-----------|--------------|-------|
| issue-3 (M1, task 3) | Introduced `Entity`, `EntityId`, `EntityKind`, `Placement`, `Provenance` and `Registry` | [spec](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/), PR #7 |

[design]: https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md

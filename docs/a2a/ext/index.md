# A2A extensions

rusty-harness extends A2A 1.0 with six extensions under one URI prefix,
`https://madarauchiha-314.github.io/rusty-harness/a2a/ext/<name>/v1`. Each one is a JSON
payload on the metadata A2A already carries; its schema is generated from the Rust types
and checked in under `specs/a2a-ext/`. The agent card advertises all six; a client
activates the ones it wants with the `A2A-Extensions` request header and the server
echoes the ones it applied. A client that activates none still receives valid A2A.

| Extension | URI | Rides on |
|---|---|---|
| [Task](./task/v1) | `…/a2a/ext/task/v1` | `Task.metadata`, the first `Message.metadata`, `TaskStatus.message.metadata` |
| [Participants](./participants/v1) | `…/a2a/ext/participants/v1` | `Task.metadata`; a message's `metadata.participant` names the sender when the caller acts for one |
| [Plan](./plan/v1) | `…/a2a/ext/plan/v1` | an artifact of media type `application/vnd.rusty-harness.plan+json`, `TaskStatus.message.metadata` |
| [Channels](./channels/v1) | `…/a2a/ext/channels/v1` | `TaskStatus.message.metadata` for an ask, `Message.metadata` for the reply |
| [Commands](./commands/v1) | `…/a2a/ext/commands/v1` | the agent card's extension `params`, `Message.metadata` |
| [Voice](./voice/v1) | `…/a2a/ext/voice/v1` | the metadata of an audio `Part` |

The design's [§ A2A extensions](https://github.com/MadaraUchiha-314/rusty-harness/blob/loop/issue-3-spec/docs/specs/issue-3/design.md)
has the payload summaries side by side; [channels](../../capabilities/channels.md) and
[tasks](../../capabilities/task.md) describe the behaviour behind them.

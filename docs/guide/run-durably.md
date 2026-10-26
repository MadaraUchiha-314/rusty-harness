# Run durably

Durable execution decides what survives a crash. The harness offers three modes behind
one configuration key; the loop, the tools and the A2A surface are the same in all three.

```toml
[durable]
mode = "none"   # none | embedded | remote
```

| Mode | What runs the loop | Survives a worker crash | Use it for |
|---|---|---|---|
| `none` | this process | no (a restart resumes from the last persisted iteration) | tests, scripts, a laptop |
| `embedded` | a Temporal dev server this process starts | within the process's life only | local development of durable flows |
| `remote` | a Temporal service, Temporal Cloud included | yes: another worker picks the task up | production |

## `embedded`: the dev server

The Temporal SDK's ephemeral dev server (the `temporal` CLI) is downloaded once into
`~/.cache/rusty-harness/temporal/` and started with the harness. It is a development mode:
its state lives in memory and goes with the process. It needs the `durable-embedded`
feature, which the `cli` feature includes.

```toml
[durable]
mode = "embedded"
```

## `remote`: Temporal Cloud or your own service

Everything about the connection is a reference, never a value:

```toml
[durable]
mode = "remote"

[durable.temporal]
address = "env:TEMPORAL_ADDRESS"       # my-ns.a1b2c.tmprl.cloud:7233, TLS implied with an API key
namespace = "env:TEMPORAL_NAMESPACE"
api_key = "env:TEMPORAL_API_KEY"
task_queue = "rusty-harness"
```

With `from_env = true` the SDK's own environment loader reads `TEMPORAL_ADDRESS`,
`TEMPORAL_NAMESPACE` and `TEMPORAL_API_KEY` (and a profile file) instead of the three
references above. A service that cannot be reached at startup stops the binary with exit
code 2; nothing falls back to `none`.

Run as many processes as you like on the same task queue: each is a worker. A task's
loop is one workflow (its id is the task id); each phase of an iteration is an activity,
so a process that dies mid-call loses only that attempt. The next worker's `retry_decision`
runs the `durable.activity_failure` and `durable.activity_retry` hooks, commits a
retraction of the lost attempt (renderers drop what it had shown) and runs the phase
again.

```toml
[durable.retry]
initial = "1s"
coefficient = 2
max_interval = "100s"
max_attempts = 5
```

## What is in a workflow history

Only ids, counts and outcomes. The request, the model's turn and every tool outcome are
blobs in the persistence store, named by keys the activities exchange; a Temporal history
of a task never holds a message's content or a credential (the client refuses a payload
that would carry a resolved secret).

## Where the store lives

Workers of one harness share one persistence store. With `[persistence] store = "file"`
the directory is shared and each process re-reads the journal before it reads or commits,
so two workers on one directory see each other's commits. A production deployment on
several machines wants a shared file system for that directory.

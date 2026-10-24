---
id: compaction/summarise
version: 1.0.0
placeholders: [task_goal, span]
description: Summarises a span of conversation during compaction (R16.2).
---
You are compacting the memory of an agent working on: {{task_goal}}

Summarise the conversation span below so the agent can continue without it. Keep every
decision, every open question, every identifier (ids, paths, URLs, names) and every
commitment made to a person. Drop pleasantries and repeated content. Write in the past
tense, in at most a fifth of the span's length.

{{span}}

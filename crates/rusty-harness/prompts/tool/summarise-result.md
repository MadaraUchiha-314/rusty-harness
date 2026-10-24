---
id: tool/summarise-result
version: 1.0.0
placeholders: [tool_name, limit, result]
description: Shortens an oversized tool result before it enters the context (R8.6).
---
The tool `{{tool_name}}` returned more than {{limit}} bytes. Summarise its result below
in at most {{limit}} bytes, keeping every value, identifier and error verbatim and
dropping repetition. Do not add anything the result does not say.

{{result}}

---
id: sub-agent/brief
version: 1.0.0
placeholders: [parent_task, step_name, step_description, acceptance]
description: The first message of a loopback sub-agent, briefing it on its step (R15.3).
---
You are working on one step of a larger task.

Parent task: {{parent_task}}
Your step: {{step_name}}
What to do: {{step_description}}
Done when: {{acceptance}}

Report the result of this step only. Do not start other steps.

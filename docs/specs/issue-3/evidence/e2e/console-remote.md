## Turn 1: the TUI asks; the model asks back through A2UI

```text
┌ demo-agent · 01a1758f-45a2-75bd-89f1-4136689703eb · 01a1758f-45a2-75 ◐ input-required awaiting participant ┐
│                                                                                                            │
│ you 00:14                                                                                                  │
│   Draft the release notes for rusty-harness 0.2: the npm and PyPI packages, the Responses API, voice. Keep │
│                                                                                                            │
│ demo-agent 00:14 · asked you on channel 4ad6af4d-7bf7-5cd2-a733-eebc6a7faee1                               │
│   ╭─ a2ui · basic catalog ────────────────────────────────────────────────────────────────────────╮        │
│   │  Which audience are these notes for?                                                                   │
│   │                                                                                                        │
│   │  The tone and the level of detail depend on it. Pick one, or type another.                             │
│   │                                                                                                        │
│   │  (●) Customers   ( ) Internal engineering   ( ) Both, two sections                                     │
│   │                                                                                                        │
│   │  Something else                                                                                        │
│   │  ┌─────────────────────────────────────────────────────────────────────────────────────────            │
│   │  │                                                                                                     │
│   │  └─────────────────────────────────────────────────────────────────────────────────────────            │
│   │                                                                                                        │
│   │    Send answer     [ Skip ]                                                                            │
│   ╰───────────────────────────────────────────────────────────────────────────────────────────────╯        │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│ plan no plan yet───────────────────────────────────────────────────────────────────────────────────────────│
│ › answer above, or type a message (the task stays open)                                                    │
│                                                                                                            │
│────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│  tab  next field  ←→  choose  enter  submit  esc  back to chat                   default · medium · remote │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

## Turn 2: the form is answered; the model drafts

```text
┌ demo-agent · 01a1758f-45a2-75bd-89f1-4136689703eb · 01a1758f-45a2-75bd ◐ input-required awaiting next turn ┐
│                                                                                                            │
│ you 00:14                                                                                                  │
│   Draft the release notes for rusty-harness 0.2: the npm and PyPI packages, the Responses API, voice. Keep │
│                                                                                                            │
│ demo-agent 00:14 · asked you on channel 4ad6af4d-7bf7-5cd2-a733-eebc6a7faee1                               │
│   ╭─ a2ui · basic catalog ────────────────────────────────────────────────────────────────────────╮        │
│   │  Which audience are these notes for?                                                                   │
│   │                                                                                                        │
│   │  The tone and the level of detail depend on it. Pick one, or type another.                             │
│   │                                                                                                        │
│   │  (●) Customers   ( ) Internal engineering   ( ) Both, two sections                                     │
│   │                                                                                                        │
│   │  Something else                                                                                        │
│   │  ┌─────────────────────────────────────────────────────────────────────────────────────────            │
│   │  │                                                                                                     │
│   │  └─────────────────────────────────────────────────────────────────────────────────────────            │
│   │                                                                                                        │
│   │    Send answer     [ Skip ]                                                                            │
│   ╰───────────────────────────────────────────────────────────────────────────────────────────────╯        │
│   → answer                                                                                                 │
│                                                                                                            │
│ demo-agent 00:14                                                                                           │
│   ## rusty-harness 0.2 — Draft release notes                                                               │
│                                                                                                            │
│   rusty-harness 0.2 brings npm and PyPI packages, Responses API support, and voice capabilities to your ag │
│                                                                                                            │
│   - npm package: Get started with rusty-harness in JavaScript and TypeScript projects.                     │
│   - PyPI package: Bring rusty-harness into your Python workflows.                                          │
│   - Responses API: Build agent interactions using the Responses API.                                       │
│   - Voice: Add voice interactions to your agent experiences.                                               │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│ plan no plan yet───────────────────────────────────────────────────────────────────────────────────────────│
│ › ▏                                                                                                        │
│                                                                                                            │
│────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│  enter  send  /  commands  ^c  complete  ^x  cancel                              default · medium · remote │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

## Turn 3: the second client revises; the TUI shows it

```text
┌ demo-agent · 01a1758f-45a2-75bd-89f1-4136689703eb · 01a1758f-45a2-75bd ◐ input-required awaiting next turn ┐
│   │                                                                                                        │
│   │  (●) Customers   ( ) Internal engineering   ( ) Both, two sections                                     │
│   │                                                                                                        │
│   │  Something else                                                                                        │
│   │  ┌─────────────────────────────────────────────────────────────────────────────────────────            │
│   │  │                                                                                                     │
│   │  └─────────────────────────────────────────────────────────────────────────────────────────            │
│   │                                                                                                        │
│   │    Send answer     [ Skip ]                                                                            │
│   ╰───────────────────────────────────────────────────────────────────────────────────────────────╯        │
│   → answer                                                                                                 │
│                                                                                                            │
│ demo-agent 00:14                                                                                           │
│   ## rusty-harness 0.2 — Draft release notes                                                               │
│                                                                                                            │
│   rusty-harness 0.2 brings npm and PyPI packages, Responses API support, and voice capabilities to your ag │
│                                                                                                            │
│   - npm package: Get started with rusty-harness in JavaScript and TypeScript projects.                     │
│   - PyPI package: Bring rusty-harness into your Python workflows.                                          │
│   - Responses API: Build agent interactions using the Responses API.                                       │
│   - Voice: Add voice interactions to your agent experiences.                                               │
│                                                                                                            │
│ you 00:14                                                                                                  │
│   Shorter, please: three bullets at most.                                                                  │
│                                                                                                            │
│ demo-agent 00:14                                                                                           │
│   ### rusty-harness 0.2 — Draft                                                                            │
│                                                                                                            │
│   rusty-harness 0.2 makes it easier to build agents with new packages, API support, and voice.             │
│                                                                                                            │
│   - npm and PyPI packages for JavaScript/TypeScript and Python projects.                                   │
│   - Responses API support for agent interactions.                                                          │
│   - Voice capabilities for spoken interactions.                                                            │
│ plan no plan yet───────────────────────────────────────────────────────────────────────────────────────────│
│ › ▏                                                                                                        │
│                                                                                                            │
│────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│  enter  send  /  commands  ^c  complete  ^x  cancel                              default · medium · remote │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

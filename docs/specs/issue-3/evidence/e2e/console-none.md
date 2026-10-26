## Turn 1: the TUI asks; the model asks back through A2UI

```text
┌ demo-agent · 01a17568-cbbd-74b8-957e-31b765fe251b · 01a17568-cbbe-75 ◐ input-required awaiting participant ┐
│                                                                                                            │
│ you 23:32                                                                                                  │
│   Draft the release notes for rusty-harness 0.2: the npm and PyPI packages, the Responses API, voice. Keep │
│                                                                                                            │
│ demo-agent 23:32 · asked you on channel 2b529609-8ec9-568a-a61f-3f711e6f2d7e                               │
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
│  tab  next field  ←→  choose  enter  submit  esc  back to chat                     default · medium · none │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

## Turn 2: the form is answered; the model drafts

```text
┌ demo-agent · 01a17568-cbbd-74b8-957e-31b765fe251b · 01a17568-cbbe-750c ◐ input-required awaiting next turn ┐
│                                                                                                            │
│ you 23:32                                                                                                  │
│   Draft the release notes for rusty-harness 0.2: the npm and PyPI packages, the Responses API, voice. Keep │
│                                                                                                            │
│ demo-agent 23:32 · asked you on channel 2b529609-8ec9-568a-a61f-3f711e6f2d7e                               │
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
│ demo-agent 23:32                                                                                           │
│   ## rusty-harness 0.2 — Draft release notes                                                               │
│                                                                                                            │
│   rusty-harness 0.2 brings npm and PyPI packages, Responses API support, and voice capabilities to your ag │
│                                                                                                            │
│   - npm package: Bring rusty-harness into your JavaScript and TypeScript projects.                         │
│   - PyPI package: Add rusty-harness to your Python projects.                                               │
│   - Responses API: Build agent workflows using the Responses API.                                          │
│   - Voice: Extend your agent experiences with voice interactions.                                          │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│ plan no plan yet───────────────────────────────────────────────────────────────────────────────────────────│
│ › ▏                                                                                                        │
│                                                                                                            │
│────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│  enter  send  /  commands  ^c  complete  ^x  cancel                                default · medium · none │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

## Turn 3: the second client revises; the TUI shows it

```text
┌ demo-agent · 01a17568-cbbd-74b8-957e-31b765fe251b · 01a17568-cbbe-750c ◐ input-required awaiting next turn ┐
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
│ demo-agent 23:32                                                                                           │
│   ## rusty-harness 0.2 — Draft release notes                                                               │
│                                                                                                            │
│   rusty-harness 0.2 brings npm and PyPI packages, Responses API support, and voice capabilities to your ag │
│                                                                                                            │
│   - npm package: Bring rusty-harness into your JavaScript and TypeScript projects.                         │
│   - PyPI package: Add rusty-harness to your Python projects.                                               │
│   - Responses API: Build agent workflows using the Responses API.                                          │
│   - Voice: Extend your agent experiences with voice interactions.                                          │
│                                                                                                            │
│ you 23:32                                                                                                  │
│   Shorter, please: three bullets at most.                                                                  │
│                                                                                                            │
│ demo-agent 23:32                                                                                           │
│   rusty-harness 0.2 makes it easier to build agents across languages and voice. (Draft)                    │
│                                                                                                            │
│   - npm and PyPI packages for JavaScript, TypeScript, and Python projects.                                 │
│   - Responses API support for agent workflows.                                                             │
│   - Voice capabilities for spoken interactions.                                                            │
│ plan no plan yet───────────────────────────────────────────────────────────────────────────────────────────│
│ › ▏                                                                                                        │
│                                                                                                            │
│────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│  enter  send  /  commands  ^c  complete  ^x  cancel                                default · medium · none │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

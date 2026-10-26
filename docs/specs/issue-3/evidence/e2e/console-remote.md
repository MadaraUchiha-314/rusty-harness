## Turn 1: the TUI asks; the model asks back through A2UI

```text
┌ demo-agent · 01a1766f-f36c-77bd-a903-b63e02f87f34 · 01a1766f-f36c-77 ◐ input-required awaiting participant ┐
│                                                                                                            │
│ you 04:20                                                                                                  │
│   Draft the release notes for rusty-harness 0.2: the npm and PyPI packages, the Responses API, voice. Keep │
│                                                                                                            │
│ demo-agent 04:20 · asked you on channel a4255b5c-8318-5f4e-9a38-7c68dd600fde                               │
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
┌ demo-agent · 01a1766f-f36c-77bd-a903-b63e02f87f34 · 01a1766f-f36c-77bd ◐ input-required awaiting next turn ┐
│                                                                                                            │
│ you 04:20                                                                                                  │
│   Draft the release notes for rusty-harness 0.2: the npm and PyPI packages, the Responses API, voice. Keep │
│                                                                                                            │
│ demo-agent 04:20 · asked you on channel a4255b5c-8318-5f4e-9a38-7c68dd600fde                               │
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
│ demo-agent 04:20                                                                                           │
│   Draft — rusty-harness 0.2 brings npm and PyPI packages, Responses API support, and voice capabilities to │
│                                                                                                            │
│   - npm package: Bring rusty-harness into your JavaScript and TypeScript projects.                         │
│   - PyPI package: Use rusty-harness in your Python projects.                                               │
│   - Responses API: Build agent workflows using the Responses API.                                          │
│   - Voice: Add voice interactions to your agents.                                                          │
│                                                                                                            │
│                                                                                                            │
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
┌ demo-agent · 01a1766f-f36c-77bd-a903-b63e02f87f34 · 01a1766f-f36c-77bd ◐ input-required awaiting next turn ┐
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
│ demo-agent 04:20                                                                                           │
│   Draft — rusty-harness 0.2 brings npm and PyPI packages, Responses API support, and voice capabilities to │
│                                                                                                            │
│   - npm package: Bring rusty-harness into your JavaScript and TypeScript projects.                         │
│   - PyPI package: Use rusty-harness in your Python projects.                                               │
│   - Responses API: Build agent workflows using the Responses API.                                          │
│   - Voice: Add voice interactions to your agents.                                                          │
│                                                                                                            │
│ you 04:20                                                                                                  │
│   Shorter, please: three bullets at most.                                                                  │
│                                                                                                            │
│ demo-agent 04:20                                                                                           │
│   Draft — rusty-harness 0.2 adds more ways to build and interact with agents.                              │
│                                                                                                            │
│   - npm and PyPI packages for JavaScript/TypeScript and Python projects.                                   │
│   - Responses API support for agent workflows.                                                             │
│   - Voice capabilities for spoken interactions.                                                            │
│ plan no plan yet───────────────────────────────────────────────────────────────────────────────────────────│
│ › ▏                                                                                                        │
│                                                                                                            │
│────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│  enter  send  /  commands  ^c  complete  ^x  cancel                              default · medium · remote │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

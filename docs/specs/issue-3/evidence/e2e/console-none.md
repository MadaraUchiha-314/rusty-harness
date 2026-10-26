## Turn 1: the TUI asks; the model asks back through A2UI

```text
┌ demo-agent · 01a1766f-427c-70cf-95ef-f4d6b36ff4d8 · 01a1766f-427c-70 ◐ input-required awaiting participant ┐
│                                                                                                            │
│ you 04:19                                                                                                  │
│   Draft the release notes for rusty-harness 0.2: the npm and PyPI packages, the Responses API, voice. Keep │
│                                                                                                            │
│ demo-agent 04:19 · asked you on channel 4ff698b6-0bd2-5041-96ad-135940be648b                               │
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
┌ demo-agent · 01a1766f-427c-70cf-95ef-f4d6b36ff4d8 · 01a1766f-427c-70cf ◐ input-required awaiting next turn ┐
│                                                                                                            │
│ you 04:19                                                                                                  │
│   Draft the release notes for rusty-harness 0.2: the npm and PyPI packages, the Responses API, voice. Keep │
│                                                                                                            │
│ demo-agent 04:19 · asked you on channel 4ff698b6-0bd2-5041-96ad-135940be648b                               │
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
│ demo-agent 04:19                                                                                           │
│   Draft — rusty-harness 0.2 brings npm and PyPI packages, Responses API support, and voice capabilities.   │
│                                                                                                            │
│   - npm package: Get started with rusty-harness in JavaScript and TypeScript projects.                     │
│   - PyPI package: Bring rusty-harness into your Python workflows.                                          │
│   - Responses API: Build agent interactions using the Responses API.                                       │
│   - Voice: Explore voice-based interactions alongside text.                                                │
│                                                                                                            │
│                                                                                                            │
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
┌ demo-agent · 01a1766f-427c-70cf-95ef-f4d6b36ff4d8 · 01a1766f-427c-70cf ◐ input-required awaiting next turn ┐
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
│ demo-agent 04:19                                                                                           │
│   Draft — rusty-harness 0.2 brings npm and PyPI packages, Responses API support, and voice capabilities.   │
│                                                                                                            │
│   - npm package: Get started with rusty-harness in JavaScript and TypeScript projects.                     │
│   - PyPI package: Bring rusty-harness into your Python workflows.                                          │
│   - Responses API: Build agent interactions using the Responses API.                                       │
│   - Voice: Explore voice-based interactions alongside text.                                                │
│                                                                                                            │
│ you 04:19                                                                                                  │
│   Shorter, please: three bullets at most.                                                                  │
│                                                                                                            │
│ demo-agent 04:19                                                                                           │
│   Draft — rusty-harness 0.2 adds more ways to build and interact with agents.                              │
│                                                                                                            │
│   - Get started with npm and PyPI packages.                                                                │
│   - Build agent interactions with Responses API support.                                                   │
│   - Explore voice alongside text.                                                                          │
│ plan no plan yet───────────────────────────────────────────────────────────────────────────────────────────│
│ › ▏                                                                                                        │
│                                                                                                            │
│────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│  enter  send  /  commands  ^c  complete  ^x  cancel                                default · medium · none │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

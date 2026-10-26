# Run the TUI

`rusty-harness` opens a terminal UI as soon as the server is up: an A2A client of the
harness it just started, or of a remote one. Everything it shows comes from the same
event stream a web browser or another agent would see.

## Start it

```sh
rusty-harness                       # starts the server, opens the TUI on its loopback URL
rusty-harness --url http://host:7474/   # opens the same TUI against a remote harness
rusty-harness --no-tui              # server only, logs to standard output
```

Without a terminal (a service manager, a pipe, a test), the binary runs in server mode
on its own. The first-run lines (the configuration written, the credentials found, the
durable mode, the agent card) print before the UI takes the screen.

## The screen

```text
┌ demo-agent · task_01HZX4V3K9N2 · ctx_9a1f ──────────────────── ◐ input-required awaiting participant ┐
│ you 12:30                                                                                            │
│   Draft release notes for Q4. Keep the task open, I will iterate with you.                           │
│ demo-agent 12:33 · asked you on channel ch-7f3a                                                      │
│   ╭─ a2ui · basic catalog ─────────────────────────────────────────────────────────╮                 │
│   │  Which audience are these notes for?                                                             │
│   │  (●) Customers   ( ) Internal engineering   ( ) Both, two sections                               │
│   │  Something else                                                                                  │
│   │    Send answer     [ Skip ]                                                                      │
│   ╰────────────────────────────────────────────────────────────────────────────────╯                 │
├ plan ■ collect ■ group ▣ draft □ review ──────────────────────────────────────────────────────────────┤
│ › answer above, or type a message (the task stays open)                                              │
│  tab  next field  ←→  choose  enter  submit  esc  back to chat             gpt-6.1 · high · remote   │
└──────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Header:** the agent, the task and its context, the state (`working`,
  `input-required`, `completed`, `canceled`, `failed`) and the sub-state.
- **Conversation:** turns as markdown, an A2UI ask as a form, artifacts (images drawn
  where the terminal can, their name and alt text where it cannot), notes.
- **Plan strip:** the `plan/v1` steps — `■` done, `▣` running, `□` pending.
- **Composer:** `enter` sends, a leading registered command is highlighted and the
  matching commands are listed as you type.
- **Hints:** the keys that work right now, and the model, effort and durable mode on
  the right.

## Keys

| Key | What it does |
|---|---|
| `enter` | send the composer |
| `/` | start a command; `/model` opens the picker when the harness runs in-process |
| `^c` | complete the task, after a confirmation that names the sub-tasks the policy cancels |
| `^x` | cancel the task (`CancelTask`) |
| `^d` | leave; the task stays as it is |
| `tab`, `←→`, `enter`, `esc` | inside an A2UI form: next field, choose, submit, back to the chat |
| `↑↓` | scroll the conversation |

A message sent after a task has finished starts a new task in the same context that
references the finished one; a finished task is never reopened.

## Voice

With a microphone on the machine, space on an empty composer is push-to-talk. In a
terminal that reports key releases (kitty, WezTerm, foot, Ghostty), hold space and talk,
release to send; elsewhere press space to start and press it again to send. `esc` drops
the capture. The harness needs a way to hear, configured once:

```toml
[voice]
provider = "openai-realtime"
api_key = "env:OPENAI_API_KEY"
# voice = "marin"
```

The audio goes to the harness, never to the provider; the reply comes back in text and,
for a spoken turn, as speech through the speakers. On Linux the binary needs ALSA
(`libasound2`).

## Colours

The TUI uses the terminal's own palette. Set `NO_COLOR=1` (or `TERM=dumb`) for the
monochrome variant: the same glyphs, bold and dim only.

## Where to look next

- [Run the server](./run-the-server) for the configuration and the first run.
- [The terminal renderer](../capabilities/tui) for the capability's current behaviour
  and the snapshot tests behind it.
- [The web renderer](../capabilities/web) for the browser surface on the same harness.

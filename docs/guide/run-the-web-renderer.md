# Run the web renderer

The web renderer in `web/` is a browser chat over A2A: it talks to any rusty-harness
server through the official A2A TypeScript SDK and nothing else.

## Start it

```sh
rusty-harness --no-tui                 # the harness on http://127.0.0.1:7474/
bun run --cwd web dev                  # the renderer on http://127.0.0.1:5173/
```

Open `http://127.0.0.1:5173/?url=http://127.0.0.1:7474/`. Without `?url=` the page talks
to its own origin, which is how a deployment serves both from one host. A loopback-only
harness accepts calls from any page origin; a harness behind `[server.security]` accepts
only the origins in `[server] allow_origins`.

## Use it

- **Send:** `Enter` sends, `Shift+Enter` breaks the line, `/` opens the command palette,
  `Esc` closes it.
- **Task controls:** **Complete** asks first and names the sub-tasks the policy cancels;
  **Cancel** stops the task. A message after a finished task starts a new one that
  references it.
- **Questions:** an agent's form (A2UI) renders in the conversation; its buttons answer
  on the question's channel.

## Voice

The microphone button records; press it again to send, `Esc` drops the recording. The
browser asks for microphone access once. The recording is converted to 24 kHz PCM in the
page and sent to the harness, which transcribes it, answers, and for a spoken turn sends
the answer back as speech, played once. Configure the harness's voice with `[voice]`
(see [Run the TUI](./run-the-tui#voice)); the page never sees a provider key.

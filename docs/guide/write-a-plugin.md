# Write a plugin

A plugin is a directory in the [Agent Plugins](https://agent-plugins.org/) format. It
can carry skills, MCP servers, remote hooks, commands and prompt fragments, and
rusty-harness installs it whole or not at all.

## Layout

```text
my-plugin/
├── plugin.json                 # name, version, description
├── mcp.json                    # MCP servers (stdio or streamable HTTP)
├── skills/
│   └── greeter/
│       ├── SKILL.md            # front matter + body
│       └── references/…        # resources, loaded on demand
└── dev.rusty-harness/          # rusty-harness's own namespace
    ├── hooks.json              # remote hooks on catalogue points
    ├── commands.json           # slash commands
    └── prompts/system/base.md  # a fragment replacing a built-in prompt
```

`plugin.json`:

```json
{
  "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
  "name": "my-plugin",
  "version": "1.0.0",
  "description": "Greets people and audits tool calls."
}
```

## Skills

A skill is a `SKILL.md` whose front matter names it (the name must equal the directory)
and describes it in one sentence. The description is what the model sees until it
activates the skill; the body loads then, and anything under `references/` only when
the skill asks for it.

```markdown
---
name: greeter
description: Greets a participant by name. Use when a greeting is asked for.
---
# Greeter

Say hello using the participant's display name.
```

## MCP servers

`mcp.json` lists servers by name. A stdio server's command must live under the plugin
directory (`${PLUGIN_ROOT}`) or on `PATH`; it runs with a scrubbed environment holding
only `PATH` and the `env` you list. A streamable HTTP server must be `https`, or `http`
to loopback. Every tool the server lists registers as `<plugin>/<server>/<tool>`.

```json
{
  "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
  "mcpServers": {
    "echo": { "type": "stdio", "command": "${PLUGIN_ROOT}/bin/echo-mcp" }
  }
}
```

## Hooks and commands

Under `dev.rusty-harness/`, `hooks.json` attaches a remote hook to a catalogue point in
one of the four stages (`pre`, `guard`, `in`, `post`), reached over JSON-RPC or as a
tool on one of your MCP servers; `commands.json` declares a slash command, the roles
that may run it, and the same kind of handler.

```json
[
  { "id": "audit", "point": "tool.execute", "phase": "post",
    "transport": { "mcp": { "server": "echo" } } }
]
```

## Prompts

A markdown file under `dev.rusty-harness/prompts/` with front matter (`id`, `version`,
`placeholders`) is a prompt fragment. A fragment whose id matches a built-in, such as
`system/base`, replaces it at the plugin's version.

## Install

Point `[plugins] paths` at the parent directory, or hand the same specification to the
SDK in memory. A plugin with any invalid file is refused with the field named, and
nothing of it is registered. See the [plugins capability](../capabilities/plugins.md)
for the exact rules.

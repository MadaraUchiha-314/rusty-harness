# rusty_harness (Python)

A tiny agent harness that's blazing fast — Python bindings over the Rust core.

```sh
pip install rusty_harness          # the SDK
pip install "rusty_harness[cli]"   # plus the rusty-harness binary (TUI, server)
```

```python
import asyncio
from rusty_harness import Config, Harness

config = Config.from_dict(
    {
        "models": {
            "default": {
                "provider": "openai",
                "model": "gpt-6.1",
                "api_key": "env:OPENAI_API_KEY",
            }
        },
    }
)


async def main() -> None:
    harness = await Harness.create(config)
    request = {
        "message": {
            "messageId": "m1",
            "role": "ROLE_USER",
            "parts": [{"text": "hello"}],
        }
    }
    async for event in harness.send_message(request):
        print(
            event
        )  # {"task": …} | {"message": …} | {"statusUpdate": …} | {"artifactUpdate": …}


asyncio.run(main())

# The same without asyncio: every awaitable has a *_sync twin.
for event in Harness(config).send_message_sync(request):
    print(event)
```

`Config.from_toml(text)` reads the TOML file's text; `get_task`, `cancel_task`,
`subscribe` and `serve` complete the surface, each with its `*_sync` twin. Configuration,
requests and events are plain `dict`s typed by the `TypedDict`s in the stub
(`ConfigDict`, `SendMessageRequest`, `StreamResponse`, …); a rejected call raises
`rusty_harness.A2AError` with the A2A `code`.

See the [documentation site](https://madarauchiha-314.github.io/rusty-harness/).

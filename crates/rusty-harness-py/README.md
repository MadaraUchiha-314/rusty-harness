# rusty_harness (Python)

A tiny agent harness that's blazing fast — Python bindings over the Rust core.

```sh
pip install rusty_harness
```

```python
import asyncio
import rusty_harness

print(asyncio.run(rusty_harness.hello()))
print(rusty_harness.hello_sync())
```

See the [documentation site](https://madarauchiha-314.github.io/rusty-harness/).

# T15 — Python bindings

<!-- markdownlint-disable MD013 -->

Awaitable and `*_sync` parity, the typed config round trip, and `pyright` strict.

## maturin develop, pytest, pyright

```sh
uv run maturin develop && uv run pytest && uv run pyright
```

Outcome: **passed** in 57s, 2026-10-25, at `0a97f5b`.

```text
📖 Found type stub file at rusty_harness.pyi
🛠 Installed rusty_harness-0.1.0
collected 16 items
tests/test_harness.py ......                                             [ 37%]
tests/test_hello.py ..                                                   [ 50%]
tests/test_parity.py ........                                            [100%]
============================== 16 passed in 2.15s ==============================
0 errors, 0 warnings, 0 informations
```

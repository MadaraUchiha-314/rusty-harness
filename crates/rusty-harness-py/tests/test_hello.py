import rusty_harness

GREETING = "Hello from rusty-harness!"


async def test_hello_is_awaitable() -> None:
    assert await rusty_harness.hello() == GREETING


def test_hello_sync_blocks() -> None:
    assert rusty_harness.hello_sync() == GREETING

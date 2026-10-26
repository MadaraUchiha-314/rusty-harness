"""Feature: Packaging and SDK parity
Requirement: docs/specs/issue-3/requirements.md#R30

Scenario: The harness is usable from Python, awaitable and blocking
    Given a Config built from a dict and a fake model endpoint
    When a message is sent with `async for` and with the `*_sync` twin
    Then the reply streams back as dicts and the task completes
"""

from __future__ import annotations

import pytest

import rusty_harness
from rusty_harness import Config, Harness

Calls = list[tuple[str, str | None]]


def config(base_url: str) -> Config:
    return Config.from_dict(
        {
            "models": {
                "default": {
                    "provider": "openai",
                    "model": "gpt-6.1",
                    "api_key": "env:RH_TEST_KEY",
                    "base_url": base_url,
                }
            }
        }
    )


def request(task_id: str) -> rusty_harness.SendMessageRequest:
    return {
        "message": {
            "messageId": f"m-{task_id}",
            "taskId": task_id,
            "contextId": "c1",
            "role": "ROLE_USER",
            "parts": [{"text": "say hello"}],
        }
    }


def text(event: rusty_harness.StreamResponse) -> str:
    if "message" not in event:
        return ""
    return "".join(p["text"] for p in event["message"]["parts"] if "text" in p)


async def test_a_task_runs_awaitable(model: tuple[str, Calls]) -> None:
    base_url, calls = model
    harness = await Harness.create(config(base_url))
    events = [event async for event in harness.send_message(request("t1"))]
    assert "Hello from Python." in "".join(text(e) for e in events)
    task = await harness.get_task("t1")
    assert task["status"]["state"] == "TASK_STATE_COMPLETED"
    assert calls == [("/v1/chat/completions", "Bearer sk-test")]
    replay = [event async for event in harness.subscribe("t1")]
    assert "task" in replay[0]


def test_a_task_runs_blocking(model: tuple[str, Calls]) -> None:
    base_url, _ = model
    harness = Harness(config(base_url))
    events = list(harness.send_message_sync(request("t2")))
    assert "Hello from Python." in "".join(text(e) for e in events)
    assert harness.get_task_sync("t2")["status"]["state"] == "TASK_STATE_COMPLETED"
    assert [e for e in harness.subscribe_sync("t2")]


async def test_an_unknown_task_raises_with_the_a2a_code(
    model: tuple[str, Calls],
) -> None:
    harness = await Harness.create(config(model[0]))
    with pytest.raises(rusty_harness.A2AError) as err:
        await harness.cancel_task("missing")
    assert err.value.code == -32001
    with pytest.raises(rusty_harness.A2AError):
        harness.get_task_sync("missing")


def test_config_round_trips_and_refuses_literal_credentials() -> None:
    toml = (
        '[models.default]\nprovider = "openai"\nmodel = "gpt-6.1"\napi_key = "env:K"\n'
    )
    parsed = Config.from_toml(toml)
    assert parsed.to_dict().get("models", {})["default"]["api_key"] == "env:K"
    assert Config.from_dict(parsed.to_dict()).to_dict() == parsed.to_dict()
    with pytest.raises(ValueError, match="credential reference"):
        Config.from_toml(toml.replace("env:K", "sk-live-secret"))
    with pytest.raises(ValueError, match="unknown field"):
        Config.from_dict({"nope": 1})  # pyright: ignore[reportArgumentType]


def test_building_fails_closed_without_the_credential(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("RH_TEST_KEY", raising=False)
    with pytest.raises(ValueError, match="RH_TEST_KEY"):
        Harness(config("http://127.0.0.1:9/v1"))


async def test_serve_listens_on_loopback_and_stops(model: tuple[str, Calls]) -> None:
    harness = await Harness.create(config(model[0]))
    server = await harness.serve("127.0.0.1:0")
    assert server.address.startswith("127.0.0.1:")
    server.stop()
    with pytest.raises(ValueError, match="loopback"):
        harness.serve_sync("0.0.0.0:0")


def test_the_environment_layer_applies_between_the_file_and_code(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """R30.4: code, then environment, then file, then defaults, on every path."""
    monkeypatch.setenv("RH__AGENT__NAME", "from-env")
    from_file = Config.from_toml('[agent]\nname = "from-file"\n')
    assert from_file.to_dict().get("agent", {}).get("name") == "from-env"
    from_code = Config.from_dict({"agent": {"name": "from-code"}})
    assert from_code.to_dict().get("agent", {}).get("name") == "from-code"
    isolated = Config.from_toml('[agent]\nname = "from-file"\n', env=False)
    assert isolated.to_dict().get("agent", {}).get("name") == "from-file"

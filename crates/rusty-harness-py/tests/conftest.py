"""A fake OpenAI-compatible endpoint the harness's model calls go to."""

from __future__ import annotations

import http.server
import threading
from collections.abc import Iterator
from typing import ClassVar

import pytest

SSE = (
    b'data: {"choices":[{"delta":{"content":"Hello from "}}]}\n\n'
    b'data: {"choices":[{"delta":{"content":"Python."},"finish_reason":"stop"}]}\n\n'
    b"data: [DONE]\n\n"
)


class _Model(http.server.BaseHTTPRequestHandler):
    calls: ClassVar[list[tuple[str, str | None]]] = []

    def do_POST(self) -> None:
        length = int(self.headers.get("content-length", "0"))
        self.rfile.read(length)
        _Model.calls.append((self.path, self.headers.get("authorization")))
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(SSE)))
        self.end_headers()
        self.wfile.write(SSE)

    def log_message(self, format: str, *args: object) -> None:
        pass


@pytest.fixture
def model(
    monkeypatch: pytest.MonkeyPatch,
) -> Iterator[tuple[str, list[tuple[str, str | None]]]]:
    """The fake endpoint's base URL and its calls; the key is `env:RH_TEST_KEY`."""
    monkeypatch.setenv("RH_TEST_KEY", "sk-test")
    _Model.calls = []
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Model)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{server.server_port}/v1", _Model.calls
    server.shutdown()

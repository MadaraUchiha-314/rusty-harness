#!/usr/bin/env python3
"""TUI cold start (T7, target < 200 ms): milliseconds from exec to the first-message
prompt, in a pseudo-terminal that answers the start-up queries as a terminal would
(primary device attributes, cursor position, device status, cell size).

    scripts/cold-start.py <binary> [runs]

Runs in a throwaway HOME with a `default` model whose key reference resolves to a
placeholder: nothing is sent to a provider before the first message. Prints the median.
"""

import fcntl
import os
import pty
import select
import signal
import statistics
import struct
import sys
import tempfile
import termios
import time

REPLIES = [
    (b"\x1b[c", b"\x1b[?62;22c"),
    (b"\x1b[6n", b"\x1b[1;1R"),
    (b"\x1b[5n", b"\x1b[0n"),
    (b"\x1b[16t", b"\x1b[6;16;8t"),
]
PROMPT = b"Type a message"
CONFIG = """[models.default]
provider = "openai"
model = "gpt-6.1-sol"
api_key = "env:RH_COLD_START_KEY"
"""


def once(binary: str, home: str) -> float:
    env = dict(os.environ, HOME=home, TERM="xterm-256color", RH_COLD_START_KEY="unused")
    start = time.perf_counter()
    pid, fd = pty.fork()
    if pid == 0:
        os.execve(binary, [binary, "--bind", "127.0.0.1:0"], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    seen = b""
    answered = {query: 0 for query, _ in REPLIES}
    try:
        while time.perf_counter() - start < 10:
            ready, _, _ = select.select([fd], [], [], 0.005)
            if not ready:
                continue
            seen += os.read(fd, 65536)
            for query, reply in REPLIES:
                while answered[query] < seen.count(query):
                    os.write(fd, reply)
                    answered[query] += 1
            if PROMPT in seen:
                return (time.perf_counter() - start) * 1000
        raise SystemExit("the first-message prompt never appeared")
    finally:
        os.kill(pid, signal.SIGTERM)
        os.waitpid(pid, 0)


def main() -> None:
    binary = sys.argv[1]
    runs = int(sys.argv[2]) if len(sys.argv) > 2 else 5
    with tempfile.TemporaryDirectory() as home:
        os.makedirs(os.path.join(home, ".rusty-harness"))
        with open(os.path.join(home, ".rusty-harness", "config.toml"), "w") as f:
            f.write(CONFIG)
        samples = [once(binary, home) for _ in range(runs)]
    print(f"{statistics.median(samples):.1f}")


if __name__ == "__main__":
    main()

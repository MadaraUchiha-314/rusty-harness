"""Feature: Packaging and SDK parity
Requirement: docs/specs/issue-3/requirements.md#R30

Scenario: Every async API has a blocking twin
    Given the generated stub, the one description of the Python surface
    When every awaitable or async-iterator function and method is listed
    Then each exists at runtime and so does its `*_sync` twin

The table is read from `rusty_harness.pyi`: a new async API without a twin fails here.
"""

from __future__ import annotations

import ast
from pathlib import Path

import pytest

import rusty_harness

STUB = ast.parse((Path(__file__).parent.parent / "rusty_harness.pyi").read_text())
ASYNC = ("typing.Awaitable", "typing.AsyncIterator")


def _async_names() -> list[str]:
    names: list[str] = []

    def visit(body: list[ast.stmt], owner: str | None) -> None:
        for node in body:
            if isinstance(node, ast.ClassDef):
                visit(node.body, node.name)
            elif isinstance(node, ast.FunctionDef) and not node.name.startswith("__"):
                returns = ast.unparse(node.returns) if node.returns else ""
                if returns.startswith(ASYNC):
                    names.append(f"{owner}.{node.name}" if owner else node.name)

    visit(STUB.body, None)
    return names


PARITY = _async_names()


def _resolve(name: str) -> object:
    target: object = rusty_harness
    for part in name.split("."):
        target = getattr(target, part)
    return target


def test_the_table_is_not_empty() -> None:
    assert {"hello", "Harness.create", "Harness.send_message", "Harness.serve"} <= set(
        PARITY
    )


@pytest.mark.parametrize("name", PARITY)
def test_has_a_sync_twin(name: str) -> None:
    assert callable(_resolve(name))
    assert callable(_resolve(f"{name}_sync"))

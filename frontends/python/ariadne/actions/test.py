"""Test semantic actions. The inventory selects the runner (cargo test,
pytest, ...)."""

from __future__ import annotations

from typing import Any

from .._impl import semantic, SemanticImpl


def unit(
    subject: Any = None,
    paths: list[str] | None = None,
    args: list[str] | None = None,
    using: str | None = None,
) -> SemanticImpl:
    """Run unit tests. `using` optionally pins the runner (e.g. "cargo",
    "pytest") when the inventory offers more than one."""
    return semantic("test.unit", using=using, paths=paths, args=args)


def integration(
    subject: Any = None,
    paths: list[str] | None = None,
    args: list[str] | None = None,
    using: str | None = None,
) -> SemanticImpl:
    """Run integration tests. `using` optionally pins the runner."""
    return semantic("test.integration", using=using, paths=paths, args=args)

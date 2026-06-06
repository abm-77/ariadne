"""Forge (code-hosting platform) semantic actions. The inventory selects the
implementation (e.g. gh, the GitHub CLI)."""

from __future__ import annotations

from typing import Any

from .._impl import semantic, SemanticImpl


def github(
    tag: str | None = None,
    files: list[str] | None = None,
    notes: str | None = None,
) -> SemanticImpl:
    """Create a GitHub release."""
    return semantic("forge.github", tag=tag, files=files, notes=notes)

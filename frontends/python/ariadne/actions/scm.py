"""Source-control semantic actions. The inventory selects the implementation
(e.g. git); the workflow only expresses intent."""

from __future__ import annotations

from typing import Any

from .._impl import semantic, SemanticImpl


def checkout(src: Any = None, ref: str | None = None, depth: int | None = None) -> SemanticImpl:
    """Check out the repository source."""
    return semantic("scm.checkout", ref=ref, depth=depth)

"""Format-check semantic actions. The inventory/impl binding selects the
formatter (cargo fmt, ruff, ...)."""

from __future__ import annotations

from .._impl import semantic, SemanticImpl


def check(paths: list[str] | None = None, using: str | None = None) -> SemanticImpl:
    """Check formatting. `paths` apply to formatters that take them (e.g. ruff)."""
    return semantic("fmt.check", using=using, paths=paths)

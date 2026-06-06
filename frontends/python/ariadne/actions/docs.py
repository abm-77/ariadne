"""Documentation-generation semantic actions. The inventory/impl binding selects
the generator (cargo doc, pdoc, mkdocs, ...)."""

from __future__ import annotations

from .._impl import semantic, SemanticImpl


def generate(
    package: str | None = None,
    out: str | None = None,
    using: str | None = None,
) -> SemanticImpl:
    """Generate documentation. `package`/`out` apply to generators that take them
    (e.g. pdoc, mkdocs); cargo doc emits to target/doc."""
    return semantic("docs.generate", using=using, package=package, out=out)

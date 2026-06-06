"""Code-coverage semantic actions. The inventory/impl binding selects the tool
(cargo-llvm-cov, pytest-cov, ...)."""

from __future__ import annotations

from .._impl import semantic, SemanticImpl


def measure(
    paths: list[str] | None = None,
    package: str | None = None,
    out: str | None = None,
    using: str | None = None,
) -> SemanticImpl:
    """Measure code coverage. `paths`/`package`/`out` apply per tool."""
    return semantic("coverage.measure", using=using, paths=paths, package=package, out=out)

"""Package-publishing semantic actions. The inventory selects the publisher
(twine, gh, ...)."""

from __future__ import annotations

from typing import Any

from .._impl import semantic, SemanticImpl


def publish(
    artifact: Any = None,
    registry: str = "pypi",
    dist: str = "dist/*",
    tag: str | None = None,
) -> SemanticImpl:
    """Publish a package to a registry."""
    return semantic("package.publish", registry=registry, dist=dist, tag=tag)


def install(package: str, using: str | None = None) -> SemanticImpl:
    """Install a package via the selected manager (pip/cargo/apt/...). `package`
    may be a registry name or a local path (e.g. `dist/*.whl`)."""
    return semantic("package.install", using=using, package=package)

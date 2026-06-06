"""Build semantic actions. Workflows say *what* to build; the inventory's
available implementations (cargo, maturin, buildkit, ...) decide *how*."""

from __future__ import annotations

from typing import Any

from .._impl import semantic, SemanticImpl


def binary(
    src: Any = None,
    package: str | None = None,
    release: bool = True,
    args: list[str] | None = None,
) -> SemanticImpl:
    """Build an executable binary."""
    return semantic("build.binary", package=package, release=release, args=args)


def library(
    src: Any = None,
    package: str | None = None,
    release: bool = True,
    args: list[str] | None = None,
) -> SemanticImpl:
    """Build a library."""
    return semantic("build.library", package=package, release=release, args=args)


def python_wheel(
    src: Any = None,
    package: str | None = None,
    manifest: str | None = None,
    dir: str | None = None,
    release: bool = True,
    out: str = "dist",
) -> SemanticImpl:
    """Build a Python wheel. `dir` builds from a project directory (its pyproject
    decides packaging); otherwise the crate at `manifest` is built."""
    return semantic(
        "build.python_wheel",
        package=package,
        manifest=manifest,
        dir=dir,
        release=release,
        out=out,
    )


def container_image(src: Any = None, tag: str | None = None) -> SemanticImpl:
    """Build a container image."""
    return semantic("build.container_image", tag=tag)

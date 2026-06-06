"""Supply-chain scanning semantic actions (SBOM, vulnerability)."""

from __future__ import annotations

from typing import Any

from .._impl import semantic, SemanticImpl


def sbom(
    image: Any = None,
    format: str = "spdx-json",
    output: str = "sbom.spdx.json",
) -> SemanticImpl:
    """Generate a software bill of materials."""
    return semantic("scan.sbom", image=image, format=format, output=output)


def vulnerability(image: Any = None) -> SemanticImpl:
    """Scan for known vulnerabilities."""
    return semantic("scan.vulnerability", image=image)

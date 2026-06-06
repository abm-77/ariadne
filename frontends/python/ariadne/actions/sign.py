"""Signing semantic actions. The inventory selects the signer (e.g. cosign)."""

from __future__ import annotations

from typing import Any

from .._impl import semantic, SemanticImpl


def artifact(image: Any = None, key: Any = None) -> SemanticImpl:
    """Sign an artifact or image."""
    return semantic("sign.artifact", image=image)

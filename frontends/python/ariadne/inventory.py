from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass
class _InvActor:
    id: str
    labels: list[str]
    capabilities: list[str]
    resources: Any = None

    def to_tir(self) -> dict[str, Any]:
        d: dict[str, Any] = {"id": self.id}
        if self.labels:
            d["labels"] = list(self.labels)
        if self.capabilities:
            d["capabilities"] = list(self.capabilities)
        if self.resources is not None:
            d["resources"] = self.resources.to_tir()
        return d


@dataclass
class _InvPlacement:
    id: str
    kind: str
    access_modes: list[str]
    accessible_by: list[str]

    def to_tir(self) -> dict[str, Any]:
        d: dict[str, Any] = {"id": self.id, "kind": self.kind}
        if self.access_modes:
            d["access_modes"] = list(self.access_modes)
        if self.accessible_by:
            d["accessible_by"] = list(self.accessible_by)
        return d


@dataclass
class _InvImpl:
    id: str
    version: str | None
    prefer: bool
    deny: bool

    def to_tir(self) -> dict[str, Any]:
        d: dict[str, Any] = {"id": self.id}
        if self.version is not None:
            d["version"] = self.version
        if self.prefer:
            d["prefer"] = True
        if self.deny:
            d["deny"] = True
        return d


class Inventory:
    """Available actors, placements, and implementation technologies for a workflow.

    Pass to @workflow(inventory=...) to include in emitted TIR.
    """

    def __init__(self, id: str):
        self._id = id
        self._actors: list[_InvActor] = []
        self._placements: list[_InvPlacement] = []
        self._implementations: list[_InvImpl] = []

    def actor(
        self,
        id: str,
        selector: list[str] | None = None,
        capabilities: list[str] | None = None,
        resources: Any = None,
    ) -> "Inventory":
        """Declare an execution resource. selector maps to TIR actor labels.
        resources advertises what the actor provides for action selection."""
        self._actors.append(
            _InvActor(
                id=id,
                labels=selector or [],
                capabilities=capabilities or [],
                resources=resources,
            )
        )
        return self

    def placement(
        self,
        id: str,
        kind: str,
        access_modes: list[str] | None = None,
        accessible_by: list[str] | None = None,
    ) -> "Inventory":
        """Declare a placement provider."""
        self._placements.append(
            _InvPlacement(
                id=id,
                kind=kind,
                access_modes=access_modes or [],
                accessible_by=accessible_by or [],
            )
        )
        return self

    def use(
        self,
        id: str,
        version: str | None = None,
        channel: str | None = None,
        prefer: bool = False,
    ) -> "Inventory":
        """Declare an available implementation technology (e.g. 'git', 'maturin').

        channel is an alias for version, for tools that use channel semantics
        (e.g. rust channel='stable').
        """
        self._implementations.append(
            _InvImpl(
                id=id,
                version=version or channel,
                prefer=prefer,
                deny=False,
            )
        )
        return self

    def prefer(self, id: str, version: str | None = None) -> "Inventory":
        """Declare a preferred implementation. Biases lowering selection."""
        self._implementations.append(_InvImpl(id=id, version=version, prefer=True, deny=False))
        return self

    def deny(self, id: str) -> "Inventory":
        """Explicitly exclude an implementation from lowering selection."""
        self._implementations.append(_InvImpl(id=id, version=None, prefer=False, deny=True))
        return self

    def to_tir(self) -> dict[str, Any]:
        d: dict[str, Any] = {"id": self._id}
        if self._actors:
            d["actors"] = [a.to_tir() for a in self._actors]
        if self._placements:
            d["placements"] = [p.to_tir() for p in self._placements]
        if self._implementations:
            d["implementations"] = [i.to_tir() for i in self._implementations]
        return d

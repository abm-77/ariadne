from __future__ import annotations


class ArtifactHandle:
    """Symbolic reference to an artifact in the workflow graph."""
    __slots__ = ("_id", "_name")

    def __init__(self, id_: int, name: str):
        self._id = id_
        self._name = name

    @property
    def artifact_id(self) -> int:
        return self._id

    @property
    def name(self) -> str:
        return self._name

    def __repr__(self) -> str:
        return f"ArtifactHandle({self._name!r})"


class ActorHandle:
    """Symbolic reference to an actor in the workflow graph."""
    __slots__ = ("_id", "_name")

    def __init__(self, id_: int, name: str):
        self._id = id_
        self._name = name

    @property
    def actor_id(self) -> int:
        return self._id

    @property
    def name(self) -> str:
        return self._name

    def __repr__(self) -> str:
        return f"ActorHandle({self._name!r})"


class Outputs:
    """Multi-output bundle returned by @op calls with more than one output."""

    def __init__(self, **handles: ArtifactHandle):
        for name, handle in handles.items():
            object.__setattr__(self, name, handle)

    def __repr__(self) -> str:
        names = ", ".join(k for k in self.__dict__)
        return f"Outputs({names})"

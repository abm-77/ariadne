"""Top-level functions that operate on the current graph.

These are called from within @workflow-decorated functions to declare
actors, external artifacts, placements, and policies.
"""
from __future__ import annotations

from ._graph import current_graph
from ._handle import ArtifactHandle, ActorHandle
from ._types import _type_to_tir


def artifact(name: str, ty: type) -> ArtifactHandle:
    """Declare an externally-supplied artifact (no producer in this workflow)."""
    graph = current_graph()
    id_ = graph._add_artifact(name, _type_to_tir(ty))
    return ArtifactHandle(id_, name)


def actor(
    name: str,
    labels: list[str] | None = None,
    capabilities: list[str] | None = None,
) -> ActorHandle:
    """Declare an execution resource (runner) available in this workflow."""
    graph = current_graph()
    id_ = graph._add_actor(name, labels or [], capabilities or [])
    return ActorHandle(id_, name)


def place(handle: ArtifactHandle, strategy: object) -> None:
    """Declare a placement strategy for an artifact."""
    current_graph()._add_placement(handle.artifact_id, strategy)


def max_parallel_jobs(n: int) -> None:
    """Set the maximum number of concurrently running jobs for this workflow."""
    current_graph()._set_max_parallel_jobs(n)


class Placement:
    """Placement strategy factory matching Thread IR PlacementStrategy variants."""

    @staticmethod
    def github_artifact() -> str:
        return "GithubArtifact"

    @staticmethod
    def shared_volume(path: str) -> dict:
        return {"SharedVolume": {"path": path}}

    @staticmethod
    def persistent_cache(key: str) -> dict:
        return {"PersistentCache": {"key": key}}

    @staticmethod
    def local_path(path: str) -> dict:
        return {"LocalPath": {"path": path}}

    @staticmethod
    def oci_registry(registry: str, tag: str) -> dict:
        return {"OciRegistry": {"registry": registry, "tag": tag}}

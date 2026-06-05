"""Top-level functions that operate on the current graph.

These are called from within @workflow-decorated functions to declare
actors, external artifacts, placements, and policies.
"""
from __future__ import annotations

from typing import TYPE_CHECKING

from ._graph import current_graph
from ._handle import ArtifactHandle, ActorHandle
from ._types import _type_to_tir

if TYPE_CHECKING:
    from ._graph import WorkflowGraph


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


class Pipeline:
    """Ariadne planning pipeline.

    Wraps ariadne_core.Pipeline to operate directly on WorkflowGraph objects.

    Usage:
        graph = my_workflow()
        p = Pipeline(graph)
        print(p.validate())
        yaml = p.compile(backend="github", level=2)
    """

    def __init__(self, graph: "WorkflowGraph"):
        try:
            from ariadne_core import Pipeline as _Pipeline
        except ImportError as e:
            raise ImportError(
                "ariadne_core extension not found. "
                "Install with: maturin develop --manifest-path crates/ariadne-py/Cargo.toml"
            ) from e
        self._inner = _Pipeline(graph.emit_json())
        self._graph = graph

    def validate(self) -> list[str]:
        """Validate the workflow. Returns diagnostic strings."""
        return self._inner.validate()

    def has_errors(self) -> bool:
        """True if the workflow has validation errors."""
        return self._inner.has_errors()

    def plan(self):
        """Compute a baseline execution plan."""
        return self._inner.plan()

    def optimize(self, plan, backend: str = "local", level: int = 2):
        """Optimize a plan for the given backend and optimization level (0-3)."""
        return self._inner.optimize(plan, backend=backend, level=level)

    def emit(self, plan, backend: str = "local") -> str:
        """Emit backend-specific configuration from a plan."""
        return self._inner.emit(plan, backend=backend)

    def compile(self, backend: str = "local", level: int = 2) -> str:
        """Validate + plan + optimize + emit in one call."""
        return self._inner.compile(backend=backend, level=level)

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


_OBJECTIVES = ("critical_path", "transfer_bytes", "dollar_cost")


def objectives(*priority: str) -> None:
    """Set the optimizer's objective priority order, highest priority first.

    Valid objectives: 'critical_path' (wall-clock makespan), 'transfer_bytes'
    (bytes moved between jobs), 'dollar_cost' (machine-time spend). Plans are
    compared lexicographically in this order, so e.g. objectives('dollar_cost',
    'critical_path') tells the optimizer to trade latency for cost where the
    cost model (guided by a profile) shows a saving."""
    bad = [p for p in priority if p not in _OBJECTIVES]
    if bad:
        raise ValueError(f"unknown objective(s) {bad}; valid: {list(_OBJECTIVES)}")
    current_graph()._set_objectives(list(priority))


def _profile_json(profile: "dict | str | None") -> "str | None":
    """Normalize a profile argument to JSON for the engine: a dict is dumped, a
    str is passed through (already JSON), None means the empty profile."""
    if profile is None or isinstance(profile, str):
        return profile
    import json
    return json.dumps(profile)


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
            from .ariadne_core import Pipeline as _Pipeline
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

    def optimize(self, plan, backend: str = "local", level: int = 2, profile=None):
        """Optimize a plan for the given backend and optimization level (0-3).
        `profile` (dict or JSON str) feeds the cost model with runner costs,
        durations and artifact sizes."""
        return self._inner.optimize(plan, backend=backend, level=level, profile=_profile_json(profile))

    def emit(self, plan, backend: str = "local") -> str:
        """Emit backend-specific configuration from a plan."""
        return self._inner.emit(plan, backend=backend)

    def compile(self, backend: str = "local", level: int = 2, profile=None) -> str:
        """Validate + plan + optimize + emit in one call. `profile` (dict or JSON
        str) feeds the cost model."""
        return self._inner.compile(backend=backend, level=level, profile=_profile_json(profile))

    def run_tests(self, *cases, backend: str = "github", level: int = 2, profile=None):
        """Run @test_case-decorated cases (from ariadne.testing) against this
        workflow. Each case builds its assertions; returns a TestResults — assert
        `.passed`. A prebuilt Suite or Case may also be passed."""
        from .testing import Suite, Case, TestResults
        built: list[Case] = []
        for c in cases:
            if isinstance(c, Suite):
                built.extend(c.cases)
            elif isinstance(c, Case):
                built.append(c)
            elif callable(c):
                built.append(c())
            else:
                raise TypeError(f"expected a @test_case, Case, or Suite, got {type(c)!r}")
        rows = self._inner.run_tests(Suite(built).to_json(), backend=backend, level=level, profile=_profile_json(profile))
        return TestResults(rows)

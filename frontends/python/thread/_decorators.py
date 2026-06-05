from __future__ import annotations

import inspect
import functools
from typing import Any

from ._graph import WorkflowGraph, _current, current_graph
from ._handle import ArtifactHandle, Outputs
from ._types import Effect, EffectKind, _type_to_tir


def _impl_shell(impl: Any) -> dict | None:
    if impl is not None and hasattr(impl, "to_shell"):
        return impl.to_shell()
    return None


class Op:
    """A reusable typed operation created by the @op decorator."""

    def __init__(
        self,
        fn: Any,
        outputs: dict[str, type],
        effects: list[Effect | EffectKind],
        constraints: list[dict],
        secrets: list[str],
        opaque: bool,
    ):
        self.fn = fn
        self.name = fn.__name__
        self.sig = inspect.signature(fn)
        self.outputs = outputs
        self.effects = effects
        self.constraints = constraints
        self.secrets = secrets
        self.opaque = opaque
        functools.update_wrapper(self, fn)

    def __call__(self, *args: Any, **kwargs: Any) -> ArtifactHandle | Outputs | None:
        graph = current_graph()

        bound = self.sig.bind(*args, **kwargs)
        bound.apply_defaults()

        # Allocate output artifact handles before evaluating body so the body
        # could in principle reference them (not needed now, but forward-safe).
        output_handles: dict[str, ArtifactHandle] = {}
        for out_name, ty in self.outputs.items():
            art_name = f"{self.name}/{out_name}"
            id_ = graph._add_artifact(art_name, _type_to_tir(ty))
            output_handles[out_name] = ArtifactHandle(id_, art_name)

        # Evaluate the body to get implementation metadata.
        impl = self.fn(*args, **kwargs)
        shell = _impl_shell(impl)

        # Artifact inputs = bound arguments that are ArtifactHandle instances.
        artifact_inputs = [
            v.artifact_id
            for v in bound.arguments.values()
            if isinstance(v, ArtifactHandle)
        ]

        # Resolve effects: create or reuse effect entries in the graph.
        effect_ids: list[int] = []
        for e in self.effects:
            if isinstance(e, Effect):
                eid = graph._add_effect(e.name, e.kind.value, e.requires_approval)
            elif isinstance(e, EffectKind):
                auto_name = f"{self.name}.{e.value.lower()}"
                eid = graph._add_effect(auto_name, e.value, False)
            else:
                raise TypeError(f"effects must be Effect or EffectKind, got {e!r}")
            effect_ids.append(eid)

        rec: dict[str, Any] = {"name": self.name, "op": self.name}
        if artifact_inputs:
            rec["inputs"] = artifact_inputs
        out_ids = [h.artifact_id for h in output_handles.values()]
        if out_ids:
            rec["outputs"] = out_ids
        if effect_ids:
            rec["effects"] = effect_ids
        if self.secrets:
            rec["secrets"] = list(self.secrets)
        if self.constraints:
            rec["actor_constraints"] = list(self.constraints)
        if shell:
            rec["shell"] = shell

        action_id = graph._add_action(rec)

        for h in output_handles.values():
            graph._set_producer(h.artifact_id, action_id)

        if not output_handles:
            return None
        if len(output_handles) == 1:
            return next(iter(output_handles.values()))
        return Outputs(**output_handles)


def op(
    outputs: dict[str, type] | None = None,
    effects: list[Effect | EffectKind] | None = None,
    constraints: list[dict] | None = None,
    secrets: list[str] | None = None,
    opaque: bool = False,
):
    """Decorator factory that turns a function into a reusable typed op."""
    def decorator(fn: Any) -> Op:
        return Op(
            fn,
            outputs=outputs or {},
            effects=effects or [],
            constraints=constraints or [],
            secrets=secrets or [],
            opaque=opaque,
        )
    return decorator


def workflow(fn: Any):
    """Decorator that runs a function as a workflow graph builder.

    The decorated function is called normally; op calls inside it record
    action nodes in an implicit graph. Returns a WorkflowGraph.
    """
    @functools.wraps(fn)
    def wrapper(*args: Any, **kwargs: Any) -> WorkflowGraph:
        graph = WorkflowGraph(fn.__name__)
        token = _current.set(graph)
        try:
            fn(*args, **kwargs)
        finally:
            _current.reset(token)
        return graph
    return wrapper

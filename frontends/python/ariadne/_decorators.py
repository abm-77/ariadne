from __future__ import annotations

import inspect
import functools
from typing import Any

from ._graph import WorkflowGraph, _current, current_graph
from ._handle import ArtifactHandle, CallRef, Outputs
from ._types import (
    Consequence,
    ConsequenceKind,
    OutputDecl,
    _type_to_tir,
    _output_spec_to_tir,
    _tir_to_name,
    _Sentinel,
)


def _impl_shell(impl: Any) -> dict | None:
    if impl is not None and hasattr(impl, "to_shell"):
        return impl.to_shell()
    return None


def _is_artifact_param(param: inspect.Parameter) -> bool:
    ann = param.annotation
    return isinstance(ann, type) and isinstance(ann, _Sentinel)


def _param_ty(param: inspect.Parameter) -> str:
    ann = param.annotation
    if isinstance(ann, type) and isinstance(ann, _Sentinel):
        return _tir_to_name(ann._tir)
    return "scalar"


class ActionDef:
    """A reusable typed operation created by the @action decorator."""

    def __init__(
        self,
        fn: Any,
        outputs: dict[str, type | OutputDecl],
        consequences: list[Consequence | ConsequenceKind],
        constraints: list[dict],
        secrets: list[str],
        timeout: str | None = None,
        coordination: Any = None,
        resources: Any = None,
    ):
        self.fn = fn
        self.name = fn.__name__
        self.sig = inspect.signature(fn)
        self.outputs = outputs
        self.consequences = consequences
        self.constraints = constraints
        self.secrets = secrets
        self.timeout = timeout
        self.coordination = coordination
        self.resources = resources
        functools.update_wrapper(self, fn)

    def _register_action_def(self, graph: Any, impl: Any) -> None:
        """Register this action's definition in the graph (idempotent by id)."""
        if any(d["id"] == self.name for d in graph._action_defs):
            return

        inputs = [
            {"name": str(p.name), "ty": _param_ty(p), "kind": "artifact"}
            for p in self.sig.parameters.values()
            if _is_artifact_param(p)
        ]
        inputs += [
            {"name": str(p.name), "ty": "scalar", "kind": "scalar"}
            for p in self.sig.parameters.values()
            if not _is_artifact_param(p)
        ]
        outputs = [
            {"name": k, "ty": _output_tir_str(v), "kind": "artifact"}
            for k, v in self.outputs.items()
        ]

        impls = []
        if impl is not None and hasattr(impl, "to_tir"):
            tir = impl.to_tir()
            if isinstance(tir, list):
                impls.extend(tir)
            else:
                impls.append(tir)
        elif impl is not None and hasattr(impl, "to_shell"):
            s = impl.to_shell()
            kind = getattr(impl, "_kind", "shell")
            entry: dict = {
                "kind": kind,
                "run": s.get("script", ""),
                "capture": s.get("capture", "NoCapture"),
            }
            if s.get("env"):
                entry["env"] = s["env"]
            impls.append(entry)

        graph._action_defs.append(
            {
                "id": self.name,
                "inputs": inputs,
                "outputs": outputs,
                "consequences": [
                    e.kind.value if hasattr(e, "kind") else e.value for e in self.consequences
                ],
                "implementations": impls,
            }
        )

    def __call__(self, *args: Any, **kwargs: Any) -> "ArtifactHandle | Outputs | CallRef":
        graph = current_graph()

        # `after=[...]` is a reserved ordering hint, not an action argument: it
        # adds gate edges (no data flow) to the calls/handles it references.
        after_refs = kwargs.pop("after", None)

        bound = self.sig.bind(*args, **kwargs)
        bound.apply_defaults()

        output_handles: dict[str, ArtifactHandle] = {}
        for out_name, out_spec in self.outputs.items():
            # `action-artifact`: a single token, valid on every backend (GitHub's
            # upload-artifact rejects '/'). The names themselves are left as the
            # author wrote them; '-' is just the separator.
            art_name = f"{self.name}-{out_name}"
            if isinstance(out_spec, OutputDecl):
                ty_tir = _type_to_tir(out_spec.ty)
                path = out_spec.path_hint()
                id_ = graph._add_artifact(art_name, ty_tir, path=path, lifetime=out_spec.lifetime)
            else:
                id_ = graph._add_artifact(art_name, _type_to_tir(out_spec))
            output_handles[out_name] = ArtifactHandle(id_, art_name)

        impl = self.fn(*args, **kwargs)
        shell = _impl_shell(impl)

        self._register_action_def(graph, impl)

        artifact_inputs = [
            v.artifact_id for v in bound.arguments.values() if isinstance(v, ArtifactHandle)
        ]

        consequence_ids: list[int] = []
        for e in self.consequences:
            if isinstance(e, Consequence):
                cid = graph._add_consequence(e.name, e.kind.value, e.requires_approval)
            elif isinstance(e, ConsequenceKind):
                auto_name = f"{self.name}.{e.value.lower()}"
                cid = graph._add_consequence(auto_name, e.value, False)
            else:
                raise TypeError(f"consequences must be Consequence or ConsequenceKind, got {e!r}")
            consequence_ids.append(cid)

        after_ids = _resolve_after(graph, after_refs)

        rec: dict[str, Any] = {"name": self.name, "action": self.name}
        if artifact_inputs:
            rec["inputs"] = artifact_inputs
        if after_ids:
            rec["after"] = after_ids
        out_ids = [h.artifact_id for h in output_handles.values()]
        if out_ids:
            rec["outputs"] = out_ids
        if consequence_ids:
            rec["consequences"] = consequence_ids
        if self.secrets:
            rec["secrets"] = list(self.secrets)
        if self.constraints:
            rec["actor_constraints"] = list(self.constraints)
        if shell:
            rec["shell"] = shell
        if self.timeout:
            rec["timeout"] = self.timeout
        if self.coordination is not None:
            rec["coordination"] = self.coordination.to_tir()
        if self.resources is not None:
            rec["resources"] = self.resources.to_tir()

        action_call_id = graph._add_action_call(rec)

        for h in output_handles.values():
            graph._set_producer(h.artifact_id, action_call_id)

        if not output_handles:
            # No data handle to return, but the call is still referenceable for
            # `after=` ordering edges.
            return CallRef(action_call_id)
        if len(output_handles) == 1:
            return next(iter(output_handles.values()))
        return Outputs(**output_handles)


def _resolve_after(graph: WorkflowGraph, refs: Any) -> list[int]:
    """Resolve `after=[...]` references to action-call ids. Accepts CallRefs
    (no-output actions), artifact handles / Outputs (use their producing call),
    or a single such value."""
    if refs is None:
        return []
    if isinstance(refs, (CallRef, ArtifactHandle, Outputs)):
        refs = [refs]
    ids: list[int] = []
    for r in refs:
        if isinstance(r, CallRef):
            ids.append(r.call_id)
        elif isinstance(r, ArtifactHandle):
            cid = graph._producer_of(r.artifact_id)
            if cid is not None:
                ids.append(cid)
        elif isinstance(r, Outputs):
            for h in r.__dict__.values():
                cid = graph._producer_of(h.artifact_id)
                if cid is not None:
                    ids.append(cid)
        else:
            raise TypeError(f"after= expects a CallRef/handle/Outputs, got {type(r)!r}")
    return sorted(set(ids))


def _output_tir_str(spec: type | OutputDecl) -> str:
    """Convert an output spec (bare type or OutputDecl) to a port type-name."""
    return _tir_to_name(_output_spec_to_tir(spec))


def action(
    outputs: dict[str, type | OutputDecl] | None = None,
    consequences: list[Consequence | ConsequenceKind] | None = None,
    constraints: list[dict] | None = None,
    secrets: list[str] | None = None,
    timeout: str | None = None,
    coordination: Any = None,
    resources: Any = None,
):
    """Decorator factory that turns a function into a reusable typed action."""

    def decorator(fn: Any) -> ActionDef:
        return ActionDef(
            fn,
            outputs=outputs or {},
            consequences=consequences or [],
            constraints=constraints or [],
            secrets=secrets or [],
            timeout=timeout,
            coordination=coordination,
            resources=resources,
        )

    return decorator


def workflow(
    _fn: Any = None, *, inventory: Any = None, triggers: Any = None, coordination: Any = None
):
    """Decorator that runs a function as a workflow graph builder.

    Can be used bare (@workflow) or configured
    (@workflow(inventory=inv, triggers=[on.push(...)])). The decorated function
    is called normally; action calls inside it record action nodes in an
    implicit graph. Returns a WorkflowGraph.
    """

    def decorator(fn: Any) -> Any:
        @functools.wraps(fn)
        def wrapper(*args: Any, **kwargs: Any) -> WorkflowGraph:
            graph = WorkflowGraph(fn.__name__)
            if inventory is not None:
                graph._set_inventory(inventory.to_tir())
            if triggers:
                graph._set_triggers([t.to_tir() for t in triggers])
            if coordination is not None:
                graph._set_coordination(coordination.to_tir())
            token = _current.set(graph)
            try:
                fn(*args, **kwargs)
            finally:
                _current.reset(token)
            return graph

        return wrapper

    if _fn is not None:
        return decorator(_fn)
    return decorator

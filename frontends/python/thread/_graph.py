from __future__ import annotations

import contextvars
from typing import Any

_current: contextvars.ContextVar[WorkflowGraph] = contextvars.ContextVar("_current_graph")


def current_graph() -> WorkflowGraph:
    try:
        return _current.get()
    except LookupError:
        raise RuntimeError(
            "No active workflow graph. Call within an @workflow function "
            "or use WorkflowGraph as a context manager."
        )


class WorkflowGraph:
    def __init__(self, name: str):
        self.name = name
        self._artifacts: list[dict[str, Any]] = []
        self._actions: list[dict[str, Any]] = []
        self._effects: list[dict[str, Any]] = []
        self._placements: list[dict[str, Any]] = []
        self._actors: list[dict[str, Any]] = []
        self._policies: dict[str, Any] = {}
        self._op_definitions: list[dict[str, Any]] = []
        self._token = None

    def __enter__(self) -> WorkflowGraph:
        self._token = _current.set(self)
        return self

    def __exit__(self, *_: Any) -> None:
        if self._token is not None:
            _current.reset(self._token)
            self._token = None

    # ------------------------------------------------------------------ #
    # Internal builders                                                    #
    # ------------------------------------------------------------------ #

    def _add_artifact(self, name: str, ty_tir: object) -> int:
        id_ = len(self._artifacts)
        self._artifacts.append({"name": name, "ty": ty_tir, "_producer": None})
        return id_

    def _set_producer(self, artifact_id: int, action_id: int) -> None:
        self._artifacts[artifact_id]["_producer"] = action_id

    def _add_effect(self, name: str, kind_str: str, requires_approval: bool) -> int:
        for i, e in enumerate(self._effects):
            if e["name"] == name:
                return i
        id_ = len(self._effects)
        self._effects.append({"name": name, "kind": kind_str, "requires_approval": requires_approval})
        return id_

    def _add_action(self, rec: dict[str, Any]) -> int:
        id_ = len(self._actions)
        self._actions.append(rec)
        return id_

    def _add_actor(self, name: str, labels: list[str], capabilities: list[str]) -> int:
        id_ = len(self._actors)
        entry: dict[str, Any] = {"name": name, "labels": labels}
        if capabilities:
            entry["capabilities"] = capabilities
        self._actors.append(entry)
        return id_

    def _add_placement(self, artifact_id: int, strategy: object) -> None:
        self._placements.append({"artifact": artifact_id, "strategy": strategy})

    def _set_max_parallel_jobs(self, n: int) -> None:
        self._policies["max_parallel_jobs"] = n

    def emit_json(self, indent: int = 2) -> str:
        from ._emit import emit_json
        return emit_json(self, indent=indent)

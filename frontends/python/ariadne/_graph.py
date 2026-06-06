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
        self._action_calls: list[dict[str, Any]] = []
        self._consequences: list[dict[str, Any]] = []
        self._placements: list[dict[str, Any]] = []
        self._inline_actors: list[dict[str, Any]] = []
        self._inventory: dict[str, Any] | None = None
        self._triggers: list[dict[str, Any]] = []
        self._coordination: dict[str, Any] | None = None
        self._policies: dict[str, Any] = {}
        self._action_defs: list[dict[str, Any]] = []
        self._token = None

    def __enter__(self) -> WorkflowGraph:
        self._token = _current.set(self)
        return self

    def __exit__(self, *_: Any) -> None:
        if self._token is not None:
            _current.reset(self._token)
            self._token = None

    def _add_artifact(self, name: str, ty_tir: object, path: str | None = None,
                      lifetime: str | None = None) -> int:
        id_ = len(self._artifacts)
        entry: dict = {"name": name, "ty": ty_tir, "_producer": None}
        if path is not None:
            entry["path"] = path
        if lifetime is not None:
            entry["lifetime"] = lifetime
        self._artifacts.append(entry)
        return id_

    def _set_producer(self, artifact_id: int, action_call_id: int) -> None:
        self._artifacts[artifact_id]["_producer"] = action_call_id

    def _add_consequence(self, name: str, kind_str: str, requires_approval: bool) -> int:
        for i, e in enumerate(self._consequences):
            if e["name"] == name:
                return i
        id_ = len(self._consequences)
        self._consequences.append(
            {"name": name, "kind": kind_str, "requires_approval": requires_approval}
        )
        return id_

    def _add_action_call(self, rec: dict[str, Any]) -> int:
        id_ = len(self._action_calls)
        self._action_calls.append(rec)
        return id_

    def _add_actor(self, name: str, labels: list[str], capabilities: list[str]) -> int:
        id_ = len(self._inline_actors)
        entry: dict[str, Any] = {"id": name, "labels": labels}
        if capabilities:
            entry["capabilities"] = capabilities
        self._inline_actors.append(entry)
        return id_

    def _set_inventory(self, inv_tir: dict[str, Any]) -> None:
        self._inventory = inv_tir

    def _set_triggers(self, triggers: list[dict[str, Any]]) -> None:
        self._triggers = triggers

    def _set_coordination(self, coordination: dict[str, Any]) -> None:
        self._coordination = coordination

    def _add_placement(self, artifact_id: int, strategy: object) -> None:
        self._placements.append({"artifact": artifact_id, "strategy": strategy})

    def _set_max_parallel_jobs(self, n: int) -> None:
        self._policies["max_parallel_jobs"] = n

    def _set_objectives(self, objs: list) -> None:
        self._policies["objectives"] = objs

    def emit_json(self, indent: int = 2) -> str:
        from ._emit import emit_json

        return emit_json(self, indent=indent)

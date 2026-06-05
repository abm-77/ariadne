from __future__ import annotations

import json
from typing import Any

from ._graph import WorkflowGraph


def _clean_artifact(raw: dict[str, Any]) -> dict[str, Any]:
    out: dict[str, Any] = {"name": raw["name"], "ty": raw["ty"]}
    if raw.get("_producer") is not None:
        out["producer"] = raw["_producer"]
    if raw.get("path") is not None:
        out["path"] = raw["path"]
    return out


def _clean_action(rec: dict[str, Any]) -> dict[str, Any]:
    out: dict[str, Any] = {"name": rec["name"], "op": rec["op"]}
    for key in ("inputs", "outputs", "effects", "secrets", "actor_constraints"):
        val = rec.get(key)
        if val:
            out[key] = val
    if "shell" in rec:
        out["shell"] = rec["shell"]
    return out


def emit_json(graph: WorkflowGraph, indent: int = 2) -> str:
    """Serialize a WorkflowGraph to TIR-compatible JSON."""
    doc: dict[str, Any] = {"name": graph.name}

    if graph._artifacts:
        doc["artifacts"] = [_clean_artifact(a) for a in graph._artifacts]
    if graph._actions:
        doc["actions"] = [_clean_action(a) for a in graph._actions]
    if graph._effects:
        doc["effects"] = [
            {"name": e["name"], "kind": e["kind"], "requires_approval": e["requires_approval"]}
            for e in graph._effects
        ]
    if graph._placements:
        doc["placements"] = list(graph._placements)
    if graph._actors:
        doc["actors"] = list(graph._actors)
    if graph._policies:
        doc["policies"] = dict(graph._policies)

    return json.dumps(doc, indent=indent)

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
    if raw.get("lifetime") is not None:
        out["lifetime"] = raw["lifetime"]
    return out


def _clean_action_call(rec: dict[str, Any]) -> dict[str, Any]:
    out: dict[str, Any] = {"name": rec["name"], "action": rec["action"]}
    for key in ("inputs", "outputs", "after", "consequences", "secrets", "actor_constraints"):
        val = rec.get(key)
        if val:
            out[key] = val
    if "shell" in rec:
        out["shell"] = rec["shell"]
    for key in ("timeout", "coordination", "resources"):
        if rec.get(key) is not None:
            out[key] = rec[key]
    return out


def emit_json(graph: WorkflowGraph, indent: int = 2) -> str:
    """Serialize a WorkflowGraph to TIR-compatible JSON."""
    doc: dict[str, Any] = {"name": graph.name}

    if graph._artifacts:
        doc["artifacts"] = [_clean_artifact(a) for a in graph._artifacts]
    if graph._action_calls:
        doc["action_calls"] = [_clean_action_call(a) for a in graph._action_calls]
    if graph._consequences:
        doc["consequences"] = [
            {"name": e["name"], "kind": e["kind"], "requires_approval": e["requires_approval"]}
            for e in graph._consequences
        ]
    if graph._placements:
        doc["placements"] = list(graph._placements)
    if graph._inventory is not None:
        doc["inventory"] = graph._inventory
    elif graph._inline_actors:
        doc["inventory"] = {"id": "default", "actors": list(graph._inline_actors)}
    if graph._triggers:
        doc["triggers"] = list(graph._triggers)
    if graph._coordination is not None:
        doc["coordination"] = graph._coordination
    if graph._policies:
        doc["policies"] = dict(graph._policies)
    if graph._action_defs:
        doc["action_defs"] = [_clean_action_def(d) for d in graph._action_defs]

    return json.dumps(doc, indent=indent)


def _clean_action_def(d: dict) -> dict:
    out: dict = {"id": d["id"]}
    if d.get("inputs"):
        out["inputs"] = d["inputs"]
    if d.get("outputs"):
        out["outputs"] = d["outputs"]
    if d.get("consequences"):
        out["consequences"] = d["consequences"]
    if d.get("implementations"):
        out["implementations"] = d["implementations"]
    if d.get("metadata"):
        out["metadata"] = d["metadata"]
    return out

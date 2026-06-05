"""thread - Python frontend for Thread IR (Ariadne).

Build typed workflow graphs with decorated Python functions and serialize
them to Thread IR JSON for consumption by the Ariadne planning engine.

    from thread import op, workflow, actor, artifact, emit_json
    from thread import SourceTree, Binary, Wheel, TestReport
    from thread import Effect, EffectKind, Constraint
    from thread import container, shell
"""
from ._types import (
    EffectKind,
    SourceTree, Wheel, Binary, ContainerImage,
    Sbom, Signature, ReleaseBundle, TestReport, Model,
    Custom,
    Effect,
    Constraint,
)
from ._handle import ArtifactHandle, ActorHandle, Outputs
from ._graph import WorkflowGraph
from ._decorators import op, workflow
from ._impl import container, shell, backend_instruction, implementations
from ._emit import emit_json
from ._api import artifact, actor, place, max_parallel_jobs, Placement

__all__ = [
    # graph lifecycle
    "workflow", "op",
    "WorkflowGraph",
    # artifact types
    "SourceTree", "Wheel", "Binary", "ContainerImage",
    "Sbom", "Signature", "ReleaseBundle", "TestReport", "Model",
    "Custom",
    # effect + constraint helpers
    "Effect", "EffectKind", "Constraint",
    # in-workflow builders
    "artifact", "actor", "place", "max_parallel_jobs", "Placement",
    # implementation descriptors
    "container", "shell", "backend_instruction", "implementations",
    # handles (for type hints)
    "ArtifactHandle", "ActorHandle", "Outputs",
    # serialization
    "emit_json",
]

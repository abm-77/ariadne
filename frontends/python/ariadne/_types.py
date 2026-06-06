from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import TYPE_CHECKING


class ConsequenceKind(str, Enum):
    Network = "Network"
    SecretAccess = "SecretAccess"
    GitWrite = "GitWrite"
    PublishRelease = "PublishRelease"
    Deployment = "Deployment"
    CommentOnPr = "CommentOnPr"


# Capture rule types for output declarations.


@dataclass
class _FileCapture:
    path: str


@dataclass
class _DirCapture:
    path: str


@dataclass
class _GlobCapture:
    pattern: str


@dataclass
class _RefCapture:
    template: str


@dataclass
class _StdoutCapture:
    pass


class ArtifactLifetime:
    """Convenience retention categories for output `lifetime=`. Each returns the
    category string the planner/backend interprets; a raw duration ("14d", "12h")
    may also be passed directly."""

    @staticmethod
    def ephemeral() -> str:
        return "ephemeral"

    @staticmethod
    def workflow() -> str:
        return "workflow"

    @staticmethod
    def release() -> str:
        return "release"

    @staticmethod
    def permanent() -> str:
        return "permanent"


@dataclass
class OutputDecl:
    """An artifact type paired with a physical capture rule, for use in @action outputs.

    Created via type class methods: Wheel.glob("dist/*.whl"), Binary.file("target/app"), etc.
    """

    ty: type
    capture: _FileCapture | _DirCapture | _GlobCapture | _RefCapture | _StdoutCapture
    # Retention requirement: a category ("ephemeral"/"workflow"/"release"/
    # "permanent") or a duration ("14d", "12h"). None means the backend default.
    lifetime: str | None = None

    def path_hint(self) -> str | None:
        """Path or pattern for TIR Artifact.path. None for stdout capture."""
        if isinstance(self.capture, (_FileCapture, _DirCapture)):
            return self.capture.path
        if isinstance(self.capture, _GlobCapture):
            return self.capture.pattern
        if isinstance(self.capture, _RefCapture):
            return self.capture.template
        return None


class _Sentinel(type):
    """Metaclass for artifact type sentinels.

    Provides _tir for TIR type encoding and factory methods for output capture rules.
    """

    def __new__(mcs, name: str, bases: tuple, namespace: dict, tir: object = None, **kw: object):
        cls = super().__new__(mcs, name, bases, namespace)
        cls._tir = tir if tir is not None else name
        return cls

    def __init__(cls, name: str, bases: tuple, namespace: dict, tir: object = None, **kw: object):
        super().__init__(name, bases, namespace)

    def file(cls, path: str, lifetime: str | None = None) -> OutputDecl:
        return OutputDecl(cls, _FileCapture(path), lifetime)

    def dir(cls, path: str, lifetime: str | None = None) -> OutputDecl:
        return OutputDecl(cls, _DirCapture(path), lifetime)

    def glob(cls, pattern: str, lifetime: str | None = None) -> OutputDecl:
        return OutputDecl(cls, _GlobCapture(pattern), lifetime)

    def ref(cls, template: str, lifetime: str | None = None) -> OutputDecl:
        return OutputDecl(cls, _RefCapture(template), lifetime)

    def stdout(cls, lifetime: str | None = None) -> OutputDecl:
        return OutputDecl(cls, _StdoutCapture(), lifetime)


class SourceTree(metaclass=_Sentinel):
    pass


class Wheel(metaclass=_Sentinel):
    pass


class Binary(metaclass=_Sentinel):
    pass


class ContainerImage(metaclass=_Sentinel):
    pass


class Sbom(metaclass=_Sentinel):
    pass


class Signature(metaclass=_Sentinel):
    pass


class ReleaseBundle(metaclass=_Sentinel):
    pass


class TestReport(metaclass=_Sentinel):
    pass


class CoverageData(metaclass=_Sentinel):
    pass


class DocsSite(metaclass=_Sentinel):
    pass


class ProfileData(metaclass=_Sentinel):
    pass


class Model(metaclass=_Sentinel):
    pass


def Custom(name: str) -> type:
    """Return a custom artifact type sentinel with the given name."""
    return _Sentinel(name, (), {}, tir={"Custom": name})


def _type_to_tir(ty: type) -> object:
    if isinstance(ty, _Sentinel):
        return ty._tir
    raise TypeError(f"Not an artifact type: {ty!r}. Use SourceTree, Wheel, Binary, etc.")


def _tir_to_name(tir: object) -> str:
    """Canonical type-name string for an ActionDef port, matching the Rust
    ArtifactType display: a built-in's name ("Wheel") or a Custom type's name
    (from `{"Custom": "Name"}`)."""
    if isinstance(tir, dict):
        return str(next(iter(tir.values())))
    return str(tir)


def _output_spec_to_tir(spec: type | OutputDecl) -> object:
    """Get TIR type string from either a bare type or an OutputDecl."""
    if isinstance(spec, OutputDecl):
        return _type_to_tir(spec.ty)
    return _type_to_tir(spec)


@dataclass
class Consequence:
    """Named consequence declaration for use in @action."""

    name: str
    kind: ConsequenceKind
    requires_approval: bool = False


class Constraint:
    """Actor constraint factory."""

    @staticmethod
    def label(label: str) -> dict:
        return {"Label": label}

    @staticmethod
    def specific(actor_handle: "ActorHandle") -> dict:
        return {"Specific": actor_handle.actor_id}

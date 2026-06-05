from enum import Enum
from dataclasses import dataclass


class EffectKind(str, Enum):
    Network = "Network"
    SecretAccess = "SecretAccess"
    GitWrite = "GitWrite"
    PublishRelease = "PublishRelease"
    Deployment = "Deployment"
    CommentOnPr = "CommentOnPr"


class _Sentinel(type):
    """Metaclass for artifact type sentinels.

    Classes created with `metaclass=_Sentinel` gain a `_tir` attribute
    containing their Thread IR type representation.
    """

    def __new__(mcs, name: str, bases: tuple, namespace: dict, tir: object = None, **kw: object):
        cls = super().__new__(mcs, name, bases, namespace)
        cls._tir = tir if tir is not None else name
        return cls

    def __init__(cls, name: str, bases: tuple, namespace: dict, tir: object = None, **kw: object):
        super().__init__(name, bases, namespace)


class SourceTree(metaclass=_Sentinel): pass
class Wheel(metaclass=_Sentinel): pass
class Binary(metaclass=_Sentinel): pass
class ContainerImage(metaclass=_Sentinel): pass
class Sbom(metaclass=_Sentinel): pass
class Signature(metaclass=_Sentinel): pass
class ReleaseBundle(metaclass=_Sentinel): pass
class TestReport(metaclass=_Sentinel): pass
class Model(metaclass=_Sentinel): pass


def Custom(name: str) -> type:
    """Return a custom artifact type sentinel with the given name."""
    return _Sentinel(name, (), {}, tir={"Custom": name})


def _type_to_tir(ty: type) -> object:
    if isinstance(ty, _Sentinel):
        return ty._tir
    raise TypeError(f"Not an artifact type: {ty!r}. Use SourceTree, Wheel, Binary, etc.")


@dataclass
class Effect:
    """Named effect declaration for use in @op."""
    name: str
    kind: EffectKind
    requires_approval: bool = False


class Constraint:
    """Actor constraint factory."""

    @staticmethod
    def label(label: str) -> dict:
        return {"Label": label}

    @staticmethod
    def specific(actor_handle: "ActorHandle") -> dict:
        return {"Specific": actor_handle.actor_id}

"""Implementation descriptors returned from @op bodies.

These are not executed; they are captured metadata that informs
how the action's shell field (and eventually backend instructions) are filled.
"""
from __future__ import annotations


class ContainerImpl:
    def __init__(self, image: str, run: str, env: dict | None = None):
        self.image = image
        self.run = run
        self.env = env or {}

    def to_shell(self) -> dict:
        s: dict = {"script": self.run, "capture": "NoCapture"}
        if self.env:
            s["env"] = self.env
        return s


class ShellImpl:
    def __init__(self, run: str, env: dict | None = None, capture: str = "NoCapture"):
        self.run = run
        self.env = env or {}
        self.capture = capture

    def to_shell(self) -> dict:
        s: dict = {"script": self.run, "capture": self.capture}
        if self.env:
            s["env"] = self.env
        return s


class BackendInstructionImpl:
    """Opaque implementation backed by a backend-specific step (e.g. github.uses)."""

    def __init__(self, kind: str, ref: str | None = None, with_: dict | None = None):
        self.kind = kind
        self.ref = ref
        self.with_ = with_ or {}

    def to_shell(self) -> dict:
        comment = f"# backend:{self.kind}"
        if self.ref:
            comment += f" ref:{self.ref}"
        return {"script": comment, "capture": "NoCapture"}


class MultiImpl:
    """Multiple alternative implementations; instruction selection picks one."""

    def __init__(self, *impls: ContainerImpl | ShellImpl | BackendInstructionImpl):
        self.impls = list(impls)

    def to_shell(self) -> dict:
        return self.impls[0].to_shell() if self.impls else {"script": "", "capture": "NoCapture"}


def container(
    image: str,
    run: str,
    env: dict | None = None,
) -> ContainerImpl:
    """Run a shell command inside a container image."""
    return ContainerImpl(image=image, run=run, env=env)


def shell(
    run: str,
    env: dict | None = None,
    capture: str = "NoCapture",
) -> ShellImpl:
    """Run a shell command directly on the actor."""
    return ShellImpl(run=run, env=env, capture=capture)


def backend_instruction(
    kind: str,
    ref: str | None = None,
    with_: dict | None = None,
) -> BackendInstructionImpl:
    """Opaque backend-specific step (e.g. a GitHub Action ref)."""
    return BackendInstructionImpl(kind=kind, ref=ref, with_=with_)


def implementations(
    *impls: ContainerImpl | ShellImpl | BackendInstructionImpl,
) -> MultiImpl:
    """Declare multiple alternative implementations for an op."""
    return MultiImpl(*impls)

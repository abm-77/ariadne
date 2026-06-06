"""Implementation descriptors returned from @action bodies.

These are not executed; they are captured metadata that informs
how the action's shell field (and eventually backend instructions) are filled.
"""

from __future__ import annotations

import contextvars
from typing import Any


class ContainerImpl:
    _kind = "container"

    def __init__(self, image: str, run: str, env: dict | None = None):
        self.image = image
        self.run = run
        self.env = env or {}

    def to_tir(self) -> dict:
        d: dict = {"kind": "container", "image": self.image, "run": self.run}
        if self.env:
            d["env"] = self.env
        return d

    def to_shell(self) -> dict:
        s: dict = {"script": self.run, "capture": "NoCapture"}
        if self.env:
            s["env"] = self.env
        return s


class ContainerBuilder:
    """A container spec waiting for .exec(...) to supply the body."""

    def __init__(self, image: str, env: dict | None = None):
        self.image = image
        self.env = env or {}

    def exec(self, body: str | list) -> ContainerImpl:
        if isinstance(body, str):
            import textwrap

            script = textwrap.dedent(body).strip()
        elif isinstance(body, list):
            script = "\n".join(str(s).strip() for s in body)
        else:
            raise TypeError(f"exec expects str or list, got {type(body)!r}")
        return ContainerImpl(image=self.image, run=script, env=self.env)


class ShellImpl:
    _kind = "shell"

    def __init__(self, run: str, env: dict | None = None, capture: str = "NoCapture"):
        self.run = run
        self.env = env or {}
        self.capture = capture

    def to_tir(self) -> dict:
        d: dict = {"kind": "shell", "run": self.run, "capture": self.capture}
        if self.env:
            d["env"] = self.env
        return d

    def to_shell(self) -> dict:
        s: dict = {"script": self.run, "capture": self.capture}
        if self.env:
            s["env"] = self.env
        return s


def _arg_value(v: Any) -> Any:
    """Normalize a semantic-action arg for TIR. Artifact handles become their
    name so render templates can reference them; scalars/lists pass through."""
    if hasattr(v, "name") and hasattr(v, "artifact_id"):
        return v.name
    if isinstance(v, (list, tuple)):
        return [_arg_value(x) for x in v]
    return v


class SemanticImpl:
    """A high-level semantic action (e.g. build.python_wheel). Carries no command;
    Ariadne selects the concrete implementation from the inventory at plan time."""

    _kind = "semantic"

    def __init__(
        self,
        op: str,
        args: dict | None = None,
        using: str | None = None,
        prefer: list[str] | None = None,
    ):
        self.op = op
        self.args = {k: _arg_value(v) for k, v in (args or {}).items() if v is not None}
        self.using = using
        self.prefer = list(prefer or [])

    def to_tir(self) -> dict:
        d: dict = {"kind": "semantic", "op": self.op}
        if self.args:
            d["args"] = self.args
        if self.using is not None:
            d["using"] = self.using
        if self.prefer:
            d["prefer"] = self.prefer
        return d


_current_prefer: "contextvars.ContextVar[tuple[str, ...]]" = contextvars.ContextVar(
    "_current_prefer", default=()
)


class _ImplBinding:
    """Context manager pushing scoped implementation preferences. Nests; inner
    bindings take priority. Soft: each preference applies to an action only if
    that action has a lowering for it."""

    def __init__(self, names: list[str]):
        self.names = tuple(names)
        self._token = None

    def __enter__(self) -> "_ImplBinding":
        self._token = _current_prefer.set(self.names + _current_prefer.get())
        return self

    def __exit__(self, *_exc: Any) -> bool:
        _current_prefer.reset(self._token)
        self._token = None
        return False


def impl(name: str) -> _ImplBinding:
    """Scoped preference for one implementation:

    with impl("pytest"):
        test.unit(paths=["tests/"])     # lowers via pytest where applicable
    """
    return _ImplBinding([name])


def impls(names: list[str]) -> _ImplBinding:
    """Scoped preference for several implementations at once (no nesting needed):

        with impls(["cargo", "maturin", "pytest"]):
            build.binary(...); build.python_wheel(...); test.unit(...)

    Each name applies to the actions that have a lowering for it; ties go to the
    earlier name in the list. Within the block, an explicit `using=` still wins."""
    return _ImplBinding(names)


def semantic(op: str, using: str | None = None, **args: Any) -> SemanticImpl:
    """Build a semantic-action implementation descriptor. `using` pins the
    implementation for this call (hard); enclosing `impl(...)`/`impls([...])`
    blocks contribute soft, scoped preferences."""
    return SemanticImpl(op, args, using=using, prefer=list(_current_prefer.get()))


def container(
    image: str,
    run: str | None = None,
    env: dict | None = None,
) -> ContainerImpl | ContainerBuilder:
    """Run a shell command inside a container image.

    With run: container("image", run="cmd") returns ContainerImpl directly.
    Without run: container("image").exec("cmd") or container("image").exec([steps]).
    """
    if run is not None:
        return ContainerImpl(image=image, run=run, env=env)
    return ContainerBuilder(image=image, env=env)


def shell(
    run: str,
    env: dict | None = None,
    capture: str = "NoCapture",
) -> ShellImpl:
    """Run a shell command directly on the actor."""
    return ShellImpl(run=run, env=env, capture=capture)

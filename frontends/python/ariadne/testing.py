"""Author loom test cases in the frontend, the same shape as `@workflow`/`@action`:
one decorated function per case. The body calls `expect.*` to state assertions
over the plan; pass the decorated cases to `Pipeline.run_tests(...)`.

    from ariadne.testing import test_case, event, expect

    @test_case(event=event.pull_request(fork=True))
    def fork_pr_gates_deploy():
        expect.consequence_gated("ship")
        expect.secret_withheld("PYPI_TOKEN")

    @test_case
    def policy_compliance():
        expect.max_parallel_jobs(10)

    results = Pipeline(release()).run_tests(fork_pr_gates_deploy, policy_compliance)
    assert results.passed, results.report()

Plan-level assertions evaluate in-process (no containers); execution-level ones
(`run_*`, `unit_*`, `stdout_contains`) report as skipped and belong to `loom test`.
"""

from __future__ import annotations

import contextvars
import functools
import json
from dataclasses import dataclass, field
from typing import Any

_current_case: contextvars.ContextVar["Case"] = contextvars.ContextVar("_current_test_case")


class event:
    """Triggering-event fixtures for a test case. Mirrors the engine's
    EventContext: the event decides which consequences fire vs gate and whether
    secrets are available."""

    @staticmethod
    def push(branch: str = "main") -> dict:
        return {"push": {"branch": branch}}

    @staticmethod
    def pull_request(fork: bool = False) -> dict:
        return {"pull_request": {"fork": fork}}

    @staticmethod
    def tag(name: str) -> dict:
        return {"tag": {"name": name}}


def _record(assertion: dict) -> dict:
    """Append to the active case (the @case body currently running), if any.
    Returns the assertion so it can also be used as plain data."""
    case_ = _current_case.get(None)
    if case_ is not None:
        case_.assertions.append(assertion)
    return assertion


class expect:
    """Assertion builders. Inside a @case body they record into that case; they
    also return the assertion dict. Plan-level assertions are decidable from the
    plan; execution-level ones require a real run (`loom test`)."""

    # plan-level
    @staticmethod
    def artifact_produced(artifact: str) -> dict:
        return _record({"assert": "artifact_produced", "artifact": artifact})

    @staticmethod
    def has_consequence(effect: str) -> dict:
        return _record({"assert": "has_consequence", "effect": effect})

    @staticmethod
    def consequence_requires_approval(effect: str) -> dict:
        return _record({"assert": "consequence_requires_approval", "effect": effect})

    @staticmethod
    def consequence_fired(effect: str) -> dict:
        return _record({"assert": "consequence_fired", "effect": effect})

    @staticmethod
    def consequence_gated(effect: str) -> dict:
        return _record({"assert": "consequence_gated", "effect": effect})

    @staticmethod
    def secret_spoofed(secret: str) -> dict:
        return _record({"assert": "secret_spoofed", "secret": secret})

    @staticmethod
    def secret_withheld(secret: str) -> dict:
        return _record({"assert": "secret_withheld", "secret": secret})

    @staticmethod
    def transfer_used(artifact: str, kind: str) -> dict:
        return _record({"assert": "transfer_used", "artifact": artifact, "kind": kind})

    @staticmethod
    def artifact_path(artifact: str, path: str) -> dict:
        return _record({"assert": "artifact_path", "artifact": artifact, "path": path})

    @staticmethod
    def max_parallel_jobs(max: int) -> dict:
        return _record({"assert": "max_parallel_jobs", "max": max})

    @staticmethod
    def max_jobs_with_capability(capability: str, max: int) -> dict:
        return _record({"assert": "max_jobs_with_capability", "capability": capability, "max": max})

    @staticmethod
    def max_concurrent_deployments(max: int) -> dict:
        return _record({"assert": "max_concurrent_deployments", "max": max})

    @staticmethod
    def selected_instruction(op: str, instruction: str) -> dict:
        return _record({"assert": "selected_instruction", "op": op, "instruction": instruction})

    @staticmethod
    def has_warning(contains: str) -> dict:
        return _record({"assert": "has_warning", "contains": contains})

    # execution-level (run via `loom test`)
    @staticmethod
    def run_passed() -> dict:
        return _record({"assert": "run_passed"})

    @staticmethod
    def run_failed() -> dict:
        return _record({"assert": "run_failed"})

    @staticmethod
    def unit_passed(unit: str) -> dict:
        return _record({"assert": "unit_passed", "unit": unit})

    @staticmethod
    def unit_failed(unit: str) -> dict:
        return _record({"assert": "unit_failed", "unit": unit})

    @staticmethod
    def unit_skipped(unit: str) -> dict:
        return _record({"assert": "unit_skipped", "unit": unit})

    @staticmethod
    def stdout_contains(unit: str, text: str) -> dict:
        return _record({"assert": "stdout_contains", "unit": unit, "text": text})


@dataclass
class Case:
    name: str
    assertions: list[dict] = field(default_factory=list)
    event: dict | None = None
    backend: str | None = None

    def to_tir(self) -> dict[str, Any]:
        d: dict[str, Any] = {"name": self.name, "assertions": list(self.assertions)}
        if self.event is not None:
            d["event"] = self.event
        if self.backend is not None:
            d["backend"] = self.backend
        return d


@dataclass
class Suite:
    cases: list[Case] = field(default_factory=list)

    def to_tir(self) -> dict[str, Any]:
        return {"cases": [c.to_tir() for c in self.cases]}

    def to_json(self, indent: int = 2) -> str:
        return json.dumps(self.to_tir(), indent=indent)


def test_case(_fn: Any = None, *, name: str | None = None,
              event: dict | None = None, backend: str | None = None) -> Any:
    """Decorate a function as a loom test case. The body calls `expect.*` to
    state assertions over the workflow's plan for the given `event` (default:
    push to main). `backend` is needed for `selected_instruction` assertions.
    Pass decorated cases to `Pipeline.run_tests(...)`; calling one builds its
    `Case`. Mirrors the @workflow/@action decorator style."""

    def decorator(fn: Any) -> Any:
        @functools.wraps(fn)
        def build() -> Case:
            c = Case(name=name or fn.__name__, event=event, backend=backend)
            token = _current_case.set(c)
            try:
                fn()
            finally:
                _current_case.reset(token)
            return c

        build._loom_case = True
        return build

    return decorator(_fn) if _fn is not None else decorator


# `test_case` matches pytest's `test*` collection glob; this tells pytest the
# decorator itself is not a test to collect/run.
test_case.__test__ = False


@dataclass
class AssertionResult:
    case: str
    assertion: str
    status: str  # "pass" | "fail" | "skip"
    detail: str


class TestResults:
    """Outcome of running a suite. `passed` is true when no assertion failed
    (skips do not fail)."""

    def __init__(self, rows: list[tuple]):
        self.results = [AssertionResult(*r) for r in rows]

    @property
    def passed(self) -> bool:
        return all(r.status != "fail" for r in self.results)

    def failures(self) -> list[AssertionResult]:
        return [r for r in self.results if r.status == "fail"]

    def report(self) -> str:
        lines = []
        for r in self.results:
            mark = {"pass": "ok", "fail": "FAIL", "skip": "skip"}[r.status]
            line = f"  [{mark}] {r.case}: {r.assertion}"
            if r.status != "pass":
                line += f" -- {r.detail}"
            lines.append(line)
        passed = sum(1 for r in self.results if r.status == "pass")
        total = sum(1 for r in self.results if r.status != "skip")
        lines.append(f"  {passed}/{total} assertions passed")
        return "\n".join(lines)

"""Tests for the in-process Pipeline binding (ariadne_core)."""

import json
import pytest

from ariadne import (
    action,
    workflow,
    artifact,
    actor,
    shell,
    SourceTree,
    Binary,
    TestReport,
    Consequence,
    ConsequenceKind,
    Pipeline,
)


@action(outputs={"src": SourceTree})
def checkout():
    return shell("git checkout .")


@action(outputs={"binary": Binary})
def build(src: SourceTree):
    return shell("cargo build --release")


@action(outputs={"report": TestReport})
def run_tests(binary: Binary):
    return shell("cargo test")


def simple_graph():
    @workflow
    def ci():
        actor("github-ubuntu", labels=["ubuntu-latest"])
        src = checkout()
        binary = build(src)
        run_tests(binary)

    return ci()


class TestValidate:
    def test_valid_workflow_returns_no_errors(self):
        p = Pipeline(simple_graph())
        errs = [d for d in p.validate() if d.startswith("error:")]
        assert errs == []

    def test_has_errors_false_for_valid_workflow(self):
        assert not Pipeline(simple_graph()).has_errors()

    def test_action_type_mismatch_caught(self):
        @action(outputs={"binary": Binary})
        def wrong_build(binary: Binary):
            return shell("cargo build")

        @workflow
        def bad():
            actor("r", labels=["ubuntu"])
            src = checkout()
            binary = build(src)
            wrong_build(binary)

        from ariadne import ariadne_core

        tir = json.loads(simple_graph().emit_json())
        tir["action_defs"] = [
            {
                "id": "build",
                "inputs": [{"name": "src", "ty": "Wheel", "kind": "artifact"}],
                "outputs": [{"name": "binary", "ty": "Binary", "kind": "artifact"}],
            }
        ]
        p = ariadne_core.Pipeline(json.dumps(tir))
        all_diags = p.validate()
        errs = [d for d in all_diags if "[error]" in d]
        assert any(
            "TypeMismatch" in e or "ActionPortMismatch" in e or "Wheel" in e for e in errs
        ), f"Expected type mismatch error, got: {all_diags}"


class TestPlan:
    def test_plan_succeeds(self):
        p = Pipeline(simple_graph())
        plan = p.plan()
        assert plan is not None

    def test_plan_unit_count(self):
        p = Pipeline(simple_graph())
        plan = p.plan()
        assert plan.unit_count() == 2

    def test_plan_workflow_name(self):
        p = Pipeline(simple_graph())
        plan = p.plan()
        assert plan.workflow_name() == "ci"

    def test_plan_max_concurrency(self):
        p = Pipeline(simple_graph())
        plan = p.plan()
        assert plan.max_concurrency() >= 1


class TestOptimize:
    def test_optimize_returns_plan(self):
        p = Pipeline(simple_graph())
        plan = p.plan()
        opt = p.optimize(plan, backend="github", level=2)
        assert opt is not None

    def test_optimize_level_zero_same_unit_count(self):
        p = Pipeline(simple_graph())
        baseline = p.plan()
        opt = p.optimize(baseline, backend="github", level=0)
        assert opt.unit_count() == baseline.unit_count()

    def test_optimize_records_decisions(self):
        p = Pipeline(simple_graph())
        plan = p.plan()
        opt = p.optimize(plan, backend="github", level=2)
        assert isinstance(opt.optimizations(), list)

    def test_unknown_backend_raises(self):
        p = Pipeline(simple_graph())
        plan = p.plan()
        with pytest.raises(Exception):
            p.optimize(plan, backend="bogus")


class TestEmit:
    def test_emit_github_produces_yaml(self):
        p = Pipeline(simple_graph())
        plan = p.plan()
        yaml = p.emit(plan, backend="github")
        assert "jobs:" in yaml
        assert "cargo build --release" in yaml

    def test_emit_local_produces_bash(self):
        p = Pipeline(simple_graph())
        plan = p.plan()
        bash = p.emit(plan, backend="local")
        assert "cargo build" in bash

    def test_unknown_backend_raises(self):
        p = Pipeline(simple_graph())
        plan = p.plan()
        with pytest.raises(Exception):
            p.emit(plan, backend="bogus")


class TestCompile:
    def test_compile_github(self):
        p = Pipeline(simple_graph())
        yaml = p.compile(backend="github", level=2)
        assert "jobs:" in yaml

    def test_compile_local(self):
        p = Pipeline(simple_graph())
        bash = p.compile(backend="local", level=0)
        assert "cargo build" in bash


class TestActionDefs:
    def test_action_defs_emitted(self):
        g = simple_graph()
        tir = json.loads(g.emit_json())
        assert "action_defs" in tir
        ids = [d["id"] for d in tir["action_defs"]]
        assert "checkout" in ids
        assert "build" in ids
        assert "run_tests" in ids

    def test_action_def_has_ports(self):
        g = simple_graph()
        tir = json.loads(g.emit_json())
        build_def = next(d for d in tir["action_defs"] if d["id"] == "build")
        assert any(p["name"] == "src" for p in build_def.get("inputs", []))
        assert any(p["name"] == "binary" for p in build_def.get("outputs", []))

    def test_action_def_has_implementation(self):
        g = simple_graph()
        tir = json.loads(g.emit_json())
        build_def = next(d for d in tir["action_defs"] if d["id"] == "build")
        impls = build_def.get("implementations", [])
        assert len(impls) > 0
        assert "cargo build" in impls[0].get("run", "")

    def test_action_def_deduped(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            build(src)
            build(src)

        tir = json.loads(ci().emit_json())
        build_defs = [d for d in tir.get("action_defs", []) if d["id"] == "build"]
        assert len(build_defs) == 1

    def test_loom_accepts_tir_with_action_defs(self, loom_bin):
        if loom_bin is None:
            pytest.skip("loom binary not found")
        import subprocess, tempfile, os

        g = simple_graph()
        with tempfile.NamedTemporaryFile(suffix=".tir.json", mode="w", delete=False) as f:
            f.write(g.emit_json())
            tmp = f.name
        try:
            result = subprocess.run([loom_bin, "check", tmp], capture_output=True, text=True)
            assert result.returncode == 0, result.stderr
        finally:
            os.unlink(tmp)


@pytest.fixture
def loom_bin():
    import os

    repo_root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
    for rel in ("target/release/loom", "target/debug/loom"):
        path = os.path.join(repo_root, rel)
        if os.path.isfile(path) and os.access(path, os.X_OK):
            return path
    return None

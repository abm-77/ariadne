"""Tests for TIR JSON emission: schema correctness and loom compatibility."""
import json
import os
import subprocess
import tempfile

import pytest

from thread import (
    op, workflow, artifact, actor, place, max_parallel_jobs, Placement,
    shell, container,
    SourceTree, Binary, Wheel, TestReport,
    Effect, EffectKind,
    emit_json,
)


@op(outputs={"src": SourceTree})
def checkout():
    return shell("git checkout .")


@op(outputs={"binary": Binary})
def build(src: SourceTree):
    return shell("cargo build --release")


@op(outputs={"report": TestReport})
def run_tests(binary: Binary):
    return shell("cargo test")


class TestSchema:
    def test_artifact_ty_is_string_for_known_types(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            checkout()

        tir = json.loads(ci().emit_json())
        assert tir["artifacts"][0]["ty"] == "SourceTree"

    def test_artifact_producer_is_integer(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            build(src)

        tir = json.loads(ci().emit_json())
        producers = [a.get("producer") for a in tir["artifacts"] if "producer" in a]
        assert all(isinstance(p, int) for p in producers)

    def test_action_inputs_outputs_are_integers(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            build(src)

        tir = json.loads(ci().emit_json())
        build_action = tir["actions"][1]
        assert all(isinstance(i, int) for i in build_action.get("inputs", []))
        assert all(isinstance(i, int) for i in build_action.get("outputs", []))

    def test_shell_capture_field(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            checkout()

        tir = json.loads(ci().emit_json())
        assert tir["actions"][0]["shell"]["capture"] == "NoCapture"

    def test_effect_kind_is_string(self):
        @op(outputs={}, effects=[Effect("rel", EffectKind.PublishRelease)])
        def publish():
            return shell("./pub.sh")

        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            publish()

        tir = json.loads(ci().emit_json())
        assert tir["effects"][0]["kind"] == "PublishRelease"

    def test_actor_capabilities_omitted_when_empty(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            checkout()

        tir = json.loads(ci().emit_json())
        assert "capabilities" not in tir["actors"][0]

    def test_actor_capabilities_included_when_set(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"], capabilities=["mount", "gpu"])
            checkout()

        tir = json.loads(ci().emit_json())
        assert tir["actors"][0]["capabilities"] == ["mount", "gpu"]

    def test_policies_omitted_when_empty(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            checkout()

        tir = json.loads(ci().emit_json())
        assert "policies" not in tir

    def test_custom_artifact_type(self):
        from thread import Custom
        MyType = Custom("CoverageReport")

        @op(outputs={"report": MyType})
        def coverage(src: SourceTree):
            return shell("coverage run -m pytest")

        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            coverage(src)

        tir = json.loads(ci().emit_json())
        cov_art = next(a for a in tir["artifacts"] if "coverage" in a["name"])
        assert cov_art["ty"] == {"Custom": "CoverageReport"}

    def test_emit_json_function_and_method_agree(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            checkout()

        g = ci()
        assert emit_json(g) == g.emit_json()


@pytest.fixture
def loom_bin():
    candidates = [
        "target/release/loom",
        "target/debug/loom",
    ]
    repo_root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
    for rel in candidates:
        path = os.path.join(repo_root, rel)
        if os.path.isfile(path) and os.access(path, os.X_OK):
            return path
    return None


class TestLoomCompatibility:
    def test_loom_accepts_emitted_tir(self, loom_bin):
        if loom_bin is None:
            pytest.skip("loom binary not found; run `cargo build --release` first")

        @workflow
        def simple():
            actor("github-ubuntu", labels=["ubuntu-latest"])
            src = checkout()
            binary = build(src)
            run_tests(binary)

        tir_json = simple().emit_json()

        with tempfile.NamedTemporaryFile(suffix=".tir.json", mode="w", delete=False) as f:
            f.write(tir_json)
            tmp = f.name

        try:
            result = subprocess.run(
                [loom_bin, "check", tmp],
                capture_output=True, text=True,
            )
            assert result.returncode == 0, (
                f"loom check failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
            )
            assert "valid" in result.stdout.lower()
        finally:
            os.unlink(tmp)

    def test_loom_accepts_workflow_with_effects(self, loom_bin):
        if loom_bin is None:
            pytest.skip("loom binary not found; run `cargo build --release` first")

        @op(outputs={}, effects=[Effect("deploy", EffectKind.Deployment, requires_approval=True)])
        def deploy():
            return shell("kubectl apply -f deploy/")

        @workflow
        def release():
            actor("github-ubuntu", labels=["ubuntu-latest"])
            src = checkout()
            binary = build(src)
            deploy()

        with tempfile.NamedTemporaryFile(suffix=".tir.json", mode="w", delete=False) as f:
            f.write(release().emit_json())
            tmp = f.name

        try:
            result = subprocess.run(
                [loom_bin, "check", tmp],
                capture_output=True, text=True,
            )
            assert result.returncode == 0, result.stderr
        finally:
            os.unlink(tmp)

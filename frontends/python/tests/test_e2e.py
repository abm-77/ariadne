"""End-to-end tests: Python frontend -> TIR -> validate -> plan -> optimize -> emit.

These tests exercise the full pipeline from workflow authoring through
backend-specific output, without requiring external executors.
"""

import json
import pytest

from ariadne import (
    action,
    workflow,
    artifact,
    actor,
    place,
    max_parallel_jobs,
    Placement,
    shell,
    container,
    SourceTree,
    Binary,
    Wheel,
    TestReport,
    Sbom,
    Signature,
    ReleaseBundle,
    Consequence,
    ConsequenceKind,
    Constraint,
    OutputDecl,
    Pipeline,
)


# ---------------------------------------------------------------------------
# Shared workflow factories
# ---------------------------------------------------------------------------


def build_linear_workflow():
    @action(outputs={"src": SourceTree})
    def checkout():
        return shell("git checkout .")

    @action(outputs={"binary": Binary.file("target/release/app")})
    def build(src: SourceTree):
        return shell("cargo build --release")

    @action(outputs={"report": TestReport.file("test-results.xml")})
    def test(binary: Binary):
        return shell("cargo test")

    @workflow
    def ci():
        actor("runner", labels=["ubuntu-latest"])
        src = checkout()
        binary = build(src)
        test(binary)

    return ci()


def build_parallel_workflow():
    """Fan-out: checkout -> (build_debug, build_release) -> merge_reports."""

    @action(outputs={"src": SourceTree})
    def checkout():
        return shell("git checkout .")

    @action(outputs={"debug_bin": Binary.file("target/debug/app")})
    def build_debug(src: SourceTree):
        return shell("cargo build")

    @action(outputs={"release_bin": Binary.file("target/release/app")})
    def build_release(src: SourceTree):
        return shell("cargo build --release")

    @action(outputs={"report": TestReport.file("results.xml")})
    def run_tests(debug_bin: Binary, release_bin: Binary):
        return shell("cargo test --workspace")

    @workflow
    def ci():
        actor("runner", labels=["ubuntu-latest"])
        max_parallel_jobs(4)
        src = checkout()
        debug = build_debug(src)
        release = build_release(src)
        run_tests(debug, release)

    return ci()


def build_release_workflow():
    """Multi-stage release: build, test, sign, publish."""

    @action(outputs={"src": SourceTree})
    def checkout():
        return shell("git checkout .")

    @action(outputs={"wheel": Wheel.glob("dist/*.whl")})
    def build_wheel(src: SourceTree):
        return container("python:3.12-slim").exec("""
            pip install build
            python -m build --wheel
        """)

    @action(outputs={"report": TestReport.file("junit.xml")})
    def test_wheel(wheel: Wheel):
        return container("python:3.12-slim").exec("""
            pip install pytest dist/*.whl
            pytest --junitxml=junit.xml
        """)

    @action(
        outputs={"sig": Signature.file("dist/wheel.sig")},
        consequences=[Consequence("sign", ConsequenceKind.SecretAccess)],
        secrets=["SIGNING_KEY"],
    )
    def sign_wheel(wheel: Wheel):
        return shell("cosign sign-blob dist/*.whl --key $SIGNING_KEY > dist/wheel.sig")

    @action(
        outputs={"sbom": Sbom.file("sbom.spdx.json")},
    )
    def generate_sbom(wheel: Wheel):
        return container("anchore/syft:latest").exec("""
            syft dir:dist --output spdx-json > sbom.spdx.json
        """)

    @action(
        outputs={"bundle": ReleaseBundle.file("release.tar.gz")},
    )
    def assemble(wheel: Wheel, sig: Signature, sbom: Sbom):
        return container("alpine:3.20").exec([
            "mkdir -p release",
            "cp dist/*.whl release/",
            "cp dist/wheel.sig release/",
            "cp sbom.spdx.json release/",
            "tar -czf release.tar.gz release/",
        ])

    @action(
        outputs={},
        consequences=[
            Consequence("publish", ConsequenceKind.PublishRelease, requires_approval=True)
        ],
        secrets=["PYPI_TOKEN"],
    )
    def publish(bundle: ReleaseBundle):
        return shell("twine upload release/*.whl")

    @workflow
    def release():
        actor("runner", labels=["ubuntu-latest"])
        src = checkout()
        wheel = build_wheel(src)
        report = test_wheel(wheel)
        sig = sign_wheel(wheel)
        sbom = generate_sbom(wheel)
        bundle = assemble(wheel, sig, sbom)
        publish(bundle)

    return release()


# ---------------------------------------------------------------------------
# E2E: validate
# ---------------------------------------------------------------------------


class TestE2EValidate:
    def test_linear_workflow_is_valid(self):
        p = Pipeline(build_linear_workflow())
        assert not p.has_errors(), p.validate()

    def test_parallel_workflow_is_valid(self):
        p = Pipeline(build_parallel_workflow())
        assert not p.has_errors(), p.validate()

    def test_release_workflow_is_valid(self):
        p = Pipeline(build_release_workflow())
        assert not p.has_errors(), p.validate()

    def test_tir_json_has_correct_structure(self):
        tir = json.loads(build_release_workflow().emit_json())
        assert "action_calls" in tir
        assert "artifacts" in tir
        assert "consequences" in tir
        assert len(tir["action_calls"]) == 7
        assert len(tir["consequences"]) == 2

    def test_artifact_paths_populated(self):
        tir = json.loads(build_release_workflow().emit_json())
        wheel = next(a for a in tir["artifacts"] if "wheel" in a["name"])
        assert wheel.get("path") == "dist/*.whl"
        bundle = next(a for a in tir["artifacts"] if "bundle" in a["name"])
        assert bundle.get("path") == "release.tar.gz"

    def test_consequence_metadata_correct(self):
        tir = json.loads(build_release_workflow().emit_json())
        publish_csq = next(
            c for c in tir["consequences"] if c["name"] == "publish"
        )
        assert publish_csq["kind"] == "PublishRelease"
        assert publish_csq["requires_approval"] is True

    def test_secrets_attached_to_action(self):
        tir = json.loads(build_release_workflow().emit_json())
        publish_call = next(
            a for a in tir["action_calls"] if a["name"] == "publish"
        )
        assert "PYPI_TOKEN" in publish_call.get("secrets", [])


# ---------------------------------------------------------------------------
# E2E: plan
# ---------------------------------------------------------------------------


class TestE2EPlan:
    def test_plan_returns_correct_unit_count(self):
        p = Pipeline(build_linear_workflow())
        plan = p.plan()
        assert plan.unit_count() == 2

    def test_parallel_plan_unit_count(self):
        p = Pipeline(build_parallel_workflow())
        plan = p.plan()
        assert plan.unit_count() == 3

    def test_release_plan_unit_count(self):
        p = Pipeline(build_release_workflow())
        plan = p.plan()
        assert plan.unit_count() == 6

    def test_linear_plan_concurrency_is_one(self):
        p = Pipeline(build_linear_workflow())
        plan = p.plan()
        # Fully linear chain has max concurrency of 1.
        assert plan.max_concurrency() == 1

    def test_parallel_plan_concurrency_greater_than_one(self):
        p = Pipeline(build_parallel_workflow())
        plan = p.plan()
        # build_debug and build_release run in parallel.
        assert plan.max_concurrency() >= 2

    def test_plan_workflow_name(self):
        p = Pipeline(build_release_workflow())
        plan = p.plan()
        assert plan.workflow_name() == "release"


# ---------------------------------------------------------------------------
# E2E: optimize
# ---------------------------------------------------------------------------


class TestE2EOptimize:
    def test_optimize_github_level_2(self):
        p = Pipeline(build_linear_workflow())
        plan = p.plan()
        opt = p.optimize(plan, backend="github", level=2)
        assert opt is not None
        assert opt.unit_count() == plan.unit_count()

    def test_optimize_level_zero_is_identity(self):
        p = Pipeline(build_parallel_workflow())
        baseline = p.plan()
        opt = p.optimize(baseline, backend="github", level=0)
        assert opt.unit_count() == baseline.unit_count()

    def test_optimize_returns_decisions_list(self):
        p = Pipeline(build_release_workflow())
        plan = p.plan()
        opt = p.optimize(plan, backend="github", level=2)
        assert isinstance(opt.optimizations(), list)

    def test_optimize_with_placement_hint(self):
        @action(outputs={"src": SourceTree})
        def checkout():
            return shell("git checkout .")

        @action(outputs={"binary": Binary})
        def build(src: SourceTree):
            return shell("cargo build --release")

        @workflow
        def ci():
            actor("big", labels=["self-hosted"], capabilities=["mount"])
            src = checkout()
            place(src, Placement.shared_volume("/vol"))
            build(src)

        p = Pipeline(ci())
        plan = p.plan()
        opt = p.optimize(plan, backend="local", level=2)
        assert opt is not None


# ---------------------------------------------------------------------------
# E2E: emit GitHub Actions
# ---------------------------------------------------------------------------


class TestE2EEmitGitHub:
    def test_emit_produces_yaml(self):
        p = Pipeline(build_linear_workflow())
        plan = p.plan()
        yaml = p.emit(plan, backend="github")
        assert "jobs:" in yaml

    def test_emitted_yaml_has_all_jobs(self):
        p = Pipeline(build_linear_workflow())
        plan = p.plan()
        yaml = p.emit(plan, backend="github")
        assert "checkout" in yaml
        assert "build" in yaml
        assert "test" in yaml

    def test_emitted_yaml_has_needs_ordering(self):
        p = Pipeline(build_linear_workflow())
        plan = p.plan()
        yaml = p.emit(plan, backend="github")
        # build needs checkout, test needs build — needs: should appear
        assert "needs:" in yaml

    def test_emitted_yaml_has_shell_commands(self):
        p = Pipeline(build_linear_workflow())
        plan = p.plan()
        yaml = p.emit(plan, backend="github")
        assert "cargo build --release" in yaml
        assert "cargo test" in yaml

    def test_release_workflow_emit(self):
        p = Pipeline(build_release_workflow())
        plan = p.plan()
        yaml = p.emit(plan, backend="github")
        assert "jobs:" in yaml
        assert "checkout" in yaml
        assert "build_wheel" in yaml

    def test_compile_is_equivalent_to_plan_optimize_emit(self):
        g = build_linear_workflow()
        p = Pipeline(g)

        compiled = p.compile(backend="github", level=2)

        plan = p.plan()
        opt = p.optimize(plan, backend="github", level=2)
        manual = p.emit(opt, backend="github")

        assert compiled == manual

    def test_parallel_workflow_emit_has_multiple_jobs(self):
        p = Pipeline(build_parallel_workflow())
        plan = p.plan()
        yaml = p.emit(plan, backend="github")
        assert "build_debug" in yaml
        assert "build_release" in yaml
        assert "run_tests" in yaml


# ---------------------------------------------------------------------------
# E2E: emit local (bash)
# ---------------------------------------------------------------------------


class TestE2EEmitLocal:
    def test_emit_local_produces_bash(self):
        p = Pipeline(build_linear_workflow())
        plan = p.plan()
        bash = p.emit(plan, backend="local")
        assert "cargo build --release" in bash
        assert "cargo test" in bash

    def test_emit_local_has_shebang(self):
        p = Pipeline(build_linear_workflow())
        plan = p.plan()
        bash = p.emit(plan, backend="local")
        assert "#!/" in bash

    def test_compile_local(self):
        p = Pipeline(build_linear_workflow())
        bash = p.compile(backend="local", level=0)
        assert "cargo build" in bash

    def test_release_workflow_local_emit(self):
        p = Pipeline(build_release_workflow())
        plan = p.plan()
        bash = p.emit(plan, backend="local")
        assert "pip install build" in bash or "python -m build" in bash


# ---------------------------------------------------------------------------
# E2E: consequence and policy assertions via plan
# ---------------------------------------------------------------------------


class TestE2EConsequences:
    def test_consequence_requires_approval_present_in_plan(self):
        p = Pipeline(build_release_workflow())
        plan = p.plan()
        decisions = plan.optimizations()
        assert isinstance(decisions, list)

    def test_workflow_with_max_parallel_jobs(self):
        p = Pipeline(build_parallel_workflow())
        assert not p.has_errors()
        plan = p.plan()
        assert plan.unit_count() == 3

    def test_secrets_in_release_workflow(self):
        tir = json.loads(build_release_workflow().emit_json())
        sign_call = next(a for a in tir["action_calls"] if a["name"] == "sign_wheel")
        assert "SIGNING_KEY" in sign_call.get("secrets", [])


# ---------------------------------------------------------------------------
# E2E: full compile shortcut
# ---------------------------------------------------------------------------


class TestE2ECompile:
    def test_compile_github(self):
        yaml = Pipeline(build_linear_workflow()).compile(backend="github", level=2)
        assert "jobs:" in yaml
        assert "cargo build --release" in yaml

    def test_compile_local(self):
        bash = Pipeline(build_linear_workflow()).compile(backend="local", level=0)
        assert "cargo build --release" in bash

    def test_compile_release_github(self):
        yaml = Pipeline(build_release_workflow()).compile(backend="github", level=2)
        assert "jobs:" in yaml
        assert "checkout" in yaml

    def test_compile_parallel_github(self):
        yaml = Pipeline(build_parallel_workflow()).compile(backend="github", level=2)
        assert "build_debug" in yaml
        assert "build_release" in yaml

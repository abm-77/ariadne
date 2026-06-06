"""Tests for output declarations, the container.exec() escape hatch, and the
semantic action namespaces (scm/build/test/scan/sign/package/forge)."""

import json
import pytest

from ariadne import (
    action,
    workflow,
    actor,
    shell,
    container,
    SourceTree,
    Binary,
    Wheel,
    TestReport,
    ReleaseBundle,
    OutputDecl,
    scm,
    build,
    test as test_ns,
    scan,
    sign,
    package,
    forge,
)


class TestOutputDecl:
    def test_glob_creates_output_decl(self):
        decl = Wheel.glob("dist/*.whl")
        assert isinstance(decl, OutputDecl)
        assert decl.path_hint() == "dist/*.whl"

    def test_file_creates_output_decl(self):
        decl = Binary.file("target/release/app")
        assert isinstance(decl, OutputDecl)
        assert decl.path_hint() == "target/release/app"

    def test_dir_creates_output_decl(self):
        decl = TestReport.dir("test-results/")
        assert isinstance(decl, OutputDecl)
        assert decl.path_hint() == "test-results/"

    def test_ref_creates_output_decl(self):
        from ariadne import ContainerImage
        decl = ContainerImage.ref("registry.io/app:{version}")
        assert isinstance(decl, OutputDecl)
        assert decl.path_hint() == "registry.io/app:{version}"

    def test_output_decl_type_preserved(self):
        decl = Wheel.glob("dist/*.whl")
        assert decl.ty is Wheel


class TestContainerEscapeHatch:
    def test_exec_with_string_returns_container_impl(self):
        impl = container("python:3.12-slim").exec("pip install .")
        assert impl._kind == "container"
        assert impl.run == "pip install ."
        assert impl.image == "python:3.12-slim"

    def test_exec_with_string_strips_whitespace(self):
        impl = container("python:3.12-slim").exec("""
            pip install .
        """)
        assert impl.run == "pip install ."

    def test_exec_with_string_list_joins(self):
        impl = container("alpine").exec([
            "mkdir -p dist",
            "echo hello > dist/out.txt",
        ])
        assert "mkdir -p dist" in impl.run
        assert "echo hello > dist/out.txt" in impl.run

    def test_exec_with_empty_list(self):
        assert container("python:3.12-slim").exec([]).run == ""

    def test_exec_invalid_body_raises(self):
        with pytest.raises(TypeError):
            container("img").exec(42)

    def test_escape_hatch_emits_to_tir(self):
        @action(outputs={"wheel": Wheel})
        def vendor_build(src: SourceTree):
            return container("python:3.12-slim").exec("python -m build --wheel")

        @action(outputs={"src": SourceTree})
        def checkout():
            return shell("git checkout .")

        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            vendor_build(src)

        tir = json.loads(ci().emit_json())
        d = next(d for d in tir["action_defs"] if d["id"] == "vendor_build")
        impl = d["implementations"][0]
        assert impl["kind"] == "container"
        assert "python -m build" in impl["run"]


class TestSemanticActions:
    def test_scm_checkout_emits_semantic(self):
        impl = scm.checkout()
        assert impl.to_tir() == {"kind": "semantic", "op": "scm.checkout"}

    def test_build_binary_args(self):
        impl = build.binary(package="loom", release=True)
        tir = impl.to_tir()
        assert tir["op"] == "build.binary"
        assert tir["args"]["package"] == "loom"
        assert tir["args"]["release"] is True

    def test_build_python_wheel_args(self):
        impl = build.python_wheel(manifest="crates/x/Cargo.toml", out="dist")
        tir = impl.to_tir()
        assert tir["op"] == "build.python_wheel"
        assert tir["args"]["manifest"] == "crates/x/Cargo.toml"
        assert tir["args"]["out"] == "dist"

    def test_build_container_image(self):
        assert build.container_image(tag="app:1").to_tir()["op"] == "build.container_image"

    def test_test_unit(self):
        impl = test_ns.unit(args=["--workspace"])
        assert impl.to_tir()["op"] == "test.unit"
        assert impl.to_tir()["args"]["args"] == ["--workspace"]

    def test_scan_sbom(self):
        assert scan.sbom().to_tir()["op"] == "scan.sbom"

    def test_sign_artifact(self):
        assert sign.artifact().to_tir()["op"] == "sign.artifact"

    def test_package_publish(self):
        assert package.publish(registry="pypi").to_tir()["op"] == "package.publish"

    def test_forge_github(self):
        impl = forge.github(tag="v1.0.0", files=["dist/*.whl"])
        tir = impl.to_tir()
        assert tir["op"] == "forge.github"
        assert tir["args"]["files"] == ["dist/*.whl"]

    def test_none_args_omitted(self):
        impl = build.binary(package=None, release=True)
        assert "package" not in impl.to_tir()["args"]

    def test_artifact_handle_arg_becomes_name(self):
        from ariadne._handle import ArtifactHandle
        h = ArtifactHandle(0, "checkout/src")
        impl = scan.sbom(image=h)
        assert impl.to_tir()["args"]["image"] == "checkout/src"

    def test_semantic_action_emits_in_workflow(self):
        @action(outputs={"src": SourceTree})
        def checkout():
            return scm.checkout()

        @action(outputs={"bin": Binary.file("target/release/loom")})
        def compile_bin(src: SourceTree):
            return build.binary(package="loom", release=True)

        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            compile_bin(src)

        tir = json.loads(ci().emit_json())
        d = next(d for d in tir["action_defs"] if d["id"] == "compile_bin")
        impl_tir = d["implementations"][0]
        assert impl_tir["kind"] == "semantic"
        assert impl_tir["op"] == "build.binary"
        assert impl_tir["args"]["package"] == "loom"


class TestImplBinding:
    def test_per_call_using(self):
        assert test_ns.unit(using="pytest").to_tir()["using"] == "pytest"
        assert "using" not in test_ns.unit().to_tir()

    def test_block_prefers_for_multiple_calls(self):
        from ariadne import impl
        with impl("cargo"):
            a = build.binary(package="x")
            b = test_ns.unit(args=["--workspace"])
            c = build.library(package="y")
        assert a.to_tir()["prefer"] == ["cargo"]
        assert b.to_tir()["prefer"] == ["cargo"]
        assert c.to_tir()["prefer"] == ["cargo"]

    def test_impls_block_binds_several_at_once(self):
        from ariadne import impls
        with impls(["maturin", "ruff", "pytest"]):
            w = build.python_wheel(package="ariadne")
            t = test_ns.unit(paths=["tests/"])
        # Every call carries the full preference list; selection applies each
        # name only where it has a lowering.
        assert w.to_tir()["prefer"] == ["maturin", "ruff", "pytest"]
        assert t.to_tir()["prefer"] == ["maturin", "ruff", "pytest"]

    def test_explicit_using_overrides_block(self):
        from ariadne import impl
        with impl("cargo"):
            t = test_ns.unit(using="pytest", paths=["tests/"])
        # The hard pin stands; the soft preference rides along.
        assert t.to_tir()["using"] == "pytest"
        assert t.to_tir()["prefer"] == ["cargo"]

    def test_nested_blocks_restore(self):
        from ariadne import impl, impls
        with impls(["cargo", "ruff"]):
            outer = test_ns.unit()
            with impl("pytest"):
                inner = test_ns.unit()
            after = test_ns.unit()
        # Inner bindings take priority (prepended), then restore on exit.
        assert outer.to_tir()["prefer"] == ["cargo", "ruff"]
        assert inner.to_tir()["prefer"] == ["pytest", "cargo", "ruff"]
        assert after.to_tir()["prefer"] == ["cargo", "ruff"]

    def test_binding_clears_after_block(self):
        from ariadne import impl
        with impl("cargo"):
            pass
        assert "prefer" not in test_ns.unit().to_tir()

    def test_block_lowers_multiple_actions_through_pipeline(self):
        from ariadne import Inventory, Pipeline, impls
        from ariadne.testing import test_case, expect

        inv = Inventory("ci").actor("r", selector=["ubuntu"]).use("cargo").use("pytest")

        @action(outputs={"src": SourceTree})
        def checkout():
            return scm.checkout()

        @action(outputs={"bin": Binary.file("target/release/app")})
        def rust_build(src: SourceTree):
            return build.binary(package="app", release=True)

        @action(outputs={"report": TestReport.file("r.xml")})
        def rust_test(src: SourceTree, b: Binary):
            return test_ns.unit(args=["--workspace"])

        @action(outputs={"pyreport": TestReport.file("py.xml")})
        def py_test(src: SourceTree):
            return test_ns.unit(paths=["tests/"])

        @workflow(inventory=inv)
        def ci():
            src = checkout()
            # One block, two preferences; each applies where it has a lowering:
            # cargo for the rust build/test, pytest for the python test.
            with impls(["cargo", "pytest"]):
                b = rust_build(src)
                rust_test(src, b)
                py_test(src)

        @test_case(backend="github")
        def runners_disambiguated():
            expect.selected_instruction("RunShell", "github.shell.run")

        results = Pipeline(ci()).run_tests(runners_disambiguated)
        assert results.passed, results.report()
        # The emitted action defs carry the soft preference list.
        tir = json.loads(ci().emit_json())
        defs = {d["id"]: d["implementations"][0] for d in tir["action_defs"]}
        assert defs["rust_test"]["prefer"] == ["cargo", "pytest"]
        assert defs["py_test"]["prefer"] == ["cargo", "pytest"]

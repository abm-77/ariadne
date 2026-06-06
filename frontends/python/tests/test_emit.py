"""Tests for TIR JSON emission: schema correctness and loom compatibility."""

import json
import os
import subprocess
import tempfile

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
    Consequence,
    ConsequenceKind,
    emit_json,
    Inventory,
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
        build_action = tir["action_calls"][1]
        assert all(isinstance(i, int) for i in build_action.get("inputs", []))
        assert all(isinstance(i, int) for i in build_action.get("outputs", []))

    def test_shell_capture_field(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            checkout()

        tir = json.loads(ci().emit_json())
        assert tir["action_calls"][0]["shell"]["capture"] == "NoCapture"

    def test_consequence_kind_is_string(self):
        @action(outputs={}, consequences=[Consequence("rel", ConsequenceKind.PublishRelease)])
        def publish():
            return shell("./pub.sh")

        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            publish()

        tir = json.loads(ci().emit_json())
        assert tir["consequences"][0]["kind"] == "PublishRelease"

    def test_actor_capabilities_omitted_when_empty(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            checkout()

        tir = json.loads(ci().emit_json())
        assert "capabilities" not in tir["inventory"]["actors"][0]

    def test_actor_capabilities_included_when_set(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"], capabilities=["mount", "gpu"])
            checkout()

        tir = json.loads(ci().emit_json())
        assert tir["inventory"]["actors"][0]["capabilities"] == ["mount", "gpu"]

    def test_policies_omitted_when_empty(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            checkout()

        tir = json.loads(ci().emit_json())
        assert "policies" not in tir

    def test_custom_artifact_type(self):
        from ariadne import Custom

        MyType = Custom("CoverageReport")

        @action(outputs={"report": MyType})
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


class TestInventory:
    def test_inventory_decorator_emits_inventory_key(self):
        inv = Inventory("ci-fleet")
        inv.actor("big-runner", selector=["self-hosted", "ubuntu"], capabilities=["mount"])

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        assert tir["inventory"]["id"] == "ci-fleet"

    def test_inventory_actor_uses_id_field(self):
        inv = Inventory("ci-fleet")
        inv.actor("big-runner", selector=["self-hosted"], capabilities=["mount"])

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        a = tir["inventory"]["actors"][0]
        assert a["id"] == "big-runner"
        assert "name" not in a

    def test_inventory_actor_selector_maps_to_labels(self):
        inv = Inventory("ns")
        inv.actor("r", selector=["ubuntu", "large"])

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        assert tir["inventory"]["actors"][0]["labels"] == ["ubuntu", "large"]

    def test_inventory_actor_capabilities(self):
        inv = Inventory("ns")
        inv.actor(
            "r", selector=["self-hosted"], capabilities=["linux", "docker", "cache_mount_access"]
        )

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        assert tir["inventory"]["actors"][0]["capabilities"] == [
            "linux",
            "docker",
            "cache_mount_access",
        ]

    def test_inventory_actor_empty_capabilities_omitted(self):
        inv = Inventory("ns")
        inv.actor("r", selector=["ubuntu"])

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        assert "capabilities" not in tir["inventory"]["actors"][0]

    def test_inventory_placement_emitted(self):
        inv = Inventory("ns")
        inv.actor("r", selector=["self-hosted"])
        inv.placement(
            "ns-cache",
            kind="cache_volume",
            access_modes=["mount_ro", "mount_rw"],
            accessible_by=["r"],
        )

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        p = tir["inventory"]["placements"][0]
        assert p["id"] == "ns-cache"
        assert p["kind"] == "cache_volume"
        assert p["access_modes"] == ["mount_ro", "mount_rw"]
        assert p["accessible_by"] == ["r"]

    def test_inline_actor_falls_back_to_default_inventory(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            checkout()

        tir = json.loads(ci().emit_json())
        assert tir["inventory"]["id"] == "default"
        assert tir["inventory"]["actors"][0]["id"] == "r"

    def test_inventory_chaining(self):
        inv = Inventory("fleet").actor("a", selector=["ubuntu"]).actor("b", selector=["macos"])

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        assert len(tir["inventory"]["actors"]) == 2

    def test_use_adds_implementation(self):
        inv = Inventory("fleet")
        inv.actor("r", selector=["ubuntu"])
        inv.use("git")
        inv.use("maturin")

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        ids = [i["id"] for i in tir["inventory"]["implementations"]]
        assert "git" in ids
        assert "maturin" in ids

    def test_use_with_version(self):
        inv = Inventory("fleet")
        inv.actor("r", selector=["ubuntu"])
        inv.use("python", version="3.12")

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        impl = tir["inventory"]["implementations"][0]
        assert impl["id"] == "python"
        assert impl["version"] == "3.12"

    def test_use_with_channel_alias(self):
        inv = Inventory("fleet")
        inv.actor("r", selector=["ubuntu"])
        inv.use("rust", channel="stable")

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        impl = tir["inventory"]["implementations"][0]
        assert impl["id"] == "rust"
        assert impl["version"] == "stable"

    def test_use_prefer_flag(self):
        inv = Inventory("fleet")
        inv.actor("r", selector=["ubuntu"])
        inv.use("maturin", prefer=True)

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        impl = tir["inventory"]["implementations"][0]
        assert impl["prefer"] is True

    def test_prefer_method(self):
        inv = Inventory("fleet")
        inv.actor("r", selector=["ubuntu"])
        inv.use("maturin")
        inv.prefer("buildkit")

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        buildkit = next(i for i in tir["inventory"]["implementations"] if i["id"] == "buildkit")
        assert buildkit["prefer"] is True

    def test_deny_method(self):
        inv = Inventory("fleet")
        inv.actor("r", selector=["ubuntu"])
        inv.use("buildkit")
        inv.deny("docker")

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        docker = next(i for i in tir["inventory"]["implementations"] if i["id"] == "docker")
        assert docker["deny"] is True

    def test_plain_use_omits_prefer_and_deny(self):
        inv = Inventory("fleet")
        inv.actor("r", selector=["ubuntu"])
        inv.use("git")

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        impl = tir["inventory"]["implementations"][0]
        assert "prefer" not in impl
        assert "deny" not in impl

    def test_no_implementations_omits_key(self):
        inv = Inventory("fleet")
        inv.actor("r", selector=["ubuntu"])

        @workflow(inventory=inv)
        def ci():
            checkout()

        tir = json.loads(ci().emit_json())
        assert "implementations" not in tir["inventory"]

    def test_inventory_with_actors_placements_and_implementations(self):
        inv = Inventory("github-namespace")
        inv.actor(
            "namespace-ubuntu-large",
            selector=["namespace", "ubuntu", "large"],
            capabilities=["linux", "x86_64", "docker", "cache_mount_access", "high_disk"],
        )
        inv.placement(
            "namespace-cache",
            kind="cache_volume",
            access_modes=["mount_ro", "mount_rw"],
            accessible_by=["namespace-ubuntu-large"],
        )
        inv.use("git")
        inv.use("python", version="3.12")
        inv.use("rust", channel="stable")
        inv.use("maturin")
        inv.use("uv")
        inv.use("buildkit")
        inv.use("syft")
        inv.use("cosign")
        inv.use("gh")

        @workflow(inventory=inv)
        def release():
            checkout()

        tir = json.loads(release().emit_json())
        assert tir["inventory"]["id"] == "github-namespace"
        impl_ids = {i["id"] for i in tir["inventory"]["implementations"]}
        assert impl_ids >= {
            "git",
            "python",
            "rust",
            "maturin",
            "uv",
            "buildkit",
            "syft",
            "cosign",
            "gh",
        }
        python_impl = next(i for i in tir["inventory"]["implementations"] if i["id"] == "python")
        assert python_impl["version"] == "3.12"


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
                capture_output=True,
                text=True,
            )
            assert result.returncode == 0, (
                f"loom check failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
            )
            assert "valid" in result.stdout.lower()
        finally:
            os.unlink(tmp)

    def test_loom_accepts_workflow_with_consequences(self, loom_bin):
        if loom_bin is None:
            pytest.skip("loom binary not found; run `cargo build --release` first")

        @action(
            outputs={},
            consequences=[
                Consequence("deploy", ConsequenceKind.Deployment, requires_approval=True)
            ],
        )
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
                capture_output=True,
                text=True,
            )
            assert result.returncode == 0, result.stderr
        finally:
            os.unlink(tmp)

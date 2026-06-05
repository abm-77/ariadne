"""Tests for WorkflowGraph building, handle semantics, and dataflow."""
import json
import pytest

from thread import (
    op, workflow, artifact, actor, place, max_parallel_jobs, Placement,
    container, shell,
    SourceTree, Binary, Wheel, TestReport, Signature,
    Effect, EffectKind, Constraint,
    emit_json,
    WorkflowGraph,
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


@op(outputs={"wheel": Wheel})
def build_wheel(src: SourceTree, python: str = "3.12"):
    return container(image=f"python:{python}", run="python -m build --wheel")


@op(outputs={"wheel": Wheel})
def repair_wheel(wheel: Wheel):
    return shell("wheel repair dist/*.whl")


@op(outputs={"sig": Signature})
def sign_wheel(wheel: Wheel):
    return shell("cosign sign dist/*.whl")


@op(outputs={"report": TestReport})
def check_wheel(wheel: Wheel):
    return shell("pytest")


class TestSimpleWorkflow:
    def test_artifact_count(self):
        @workflow
        def ci():
            actor("github-ubuntu", labels=["ubuntu-latest"])
            src = checkout()
            binary = build(src)
            run_tests(binary)

        tir = json.loads(ci().emit_json())
        assert len(tir["artifacts"]) == 3

    def test_action_count(self):
        @workflow
        def ci():
            actor("github-ubuntu", labels=["ubuntu-latest"])
            src = checkout()
            binary = build(src)
            run_tests(binary)

        tir = json.loads(ci().emit_json())
        assert len(tir["actions"]) == 3

    def test_workflow_name(self):
        @workflow
        def my_pipeline():
            actor("r", labels=["ubuntu-latest"])
            checkout()

        tir = json.loads(my_pipeline().emit_json())
        assert tir["name"] == "my_pipeline"

    def test_actor_declared(self):
        @workflow
        def ci():
            actor("github-ubuntu", labels=["ubuntu-latest"])
            checkout()

        tir = json.loads(ci().emit_json())
        assert tir["actors"][0]["name"] == "github-ubuntu"
        assert "ubuntu-latest" in tir["actors"][0]["labels"]

    def test_topological_order(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            binary = build(src)
            run_tests(binary)

        tir = json.loads(ci().emit_json())
        names = [a["name"] for a in tir["actions"]]
        assert names.index("checkout") < names.index("build")
        assert names.index("build") < names.index("run_tests")


class TestHandleSemantics:
    def test_handle_returned(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            assert src is not None

        ci()

    def test_handle_passed_as_input(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            binary = build(src)

        tir = json.loads(ci().emit_json())
        checkout_out = tir["actions"][0]["outputs"][0]
        build_in = tir["actions"][1]["inputs"][0]
        assert checkout_out == build_in

    def test_multiple_consumers_of_same_artifact(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            wheel = build_wheel(src)
            check_wheel(wheel)
            sign_wheel(wheel)

        tir = json.loads(ci().emit_json())
        wheel_id = tir["actions"][1]["outputs"][0]

        consumers = [
            a for a in tir["actions"]
            if wheel_id in a.get("inputs", [])
        ]
        assert len(consumers) == 2

    def test_rebinding_creates_new_artifact(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            wheel = build_wheel(src)
            wheel = repair_wheel(wheel)
            check_wheel(wheel)

        tir = json.loads(ci().emit_json())
        assert len(tir["actions"]) == 4

        build_out = tir["actions"][1]["outputs"][0]
        repair_in = tir["actions"][2]["inputs"][0]
        repair_out = tir["actions"][2]["outputs"][0]
        check_in = tir["actions"][3]["inputs"][0]

        assert build_out == repair_in
        assert repair_out == check_in
        assert build_out != repair_out


class TestExternalArtifact:
    def test_external_artifact_has_no_producer(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = artifact("source", SourceTree)
            build(src)

        tir = json.loads(ci().emit_json())
        ext = next(a for a in tir["artifacts"] if a["name"] == "source")
        assert "producer" not in ext

    def test_external_artifact_used_as_input(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = artifact("source", SourceTree)
            build(src)

        tir = json.loads(ci().emit_json())
        src_id = next(i for i, a in enumerate(tir["artifacts"]) if a["name"] == "source")
        assert src_id in tir["actions"][0].get("inputs", [])


class TestScalarParameters:
    def test_scalar_param_used_in_impl(self):
        @op(outputs={"wheel": Wheel})
        def versioned_wheel(src: SourceTree, python: str = "3.12"):
            return shell(f"python{python} -m build --wheel")

        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            versioned_wheel(src, python="3.11")

        tir = json.loads(ci().emit_json())
        bw = next(a for a in tir["actions"] if a["name"] == "versioned_wheel")
        assert "3.11" in bw["shell"]["script"]


class TestEffects:
    def test_named_effect_in_tir(self):
        @op(
            outputs={"image": SourceTree},
            effects=[Effect("deploy", EffectKind.Deployment, requires_approval=True)],
        )
        def deploy_image(src: SourceTree):
            return shell("kubectl apply -f .")

        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            deploy_image(src)

        tir = json.loads(ci().emit_json())
        assert any(e["name"] == "deploy" for e in tir.get("effects", []))
        deploy_eff = next(e for e in tir["effects"] if e["name"] == "deploy")
        assert deploy_eff["kind"] == "Deployment"
        assert deploy_eff["requires_approval"] is True

    def test_auto_named_effect(self):
        @op(outputs={}, effects=[EffectKind.Network])
        def fetch_data():
            return shell("curl https://example.com")

        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            fetch_data()

        tir = json.loads(ci().emit_json())
        assert any(e["name"] == "fetch_data.network" for e in tir.get("effects", []))

    def test_effect_id_referenced_in_action(self):
        @op(
            outputs={},
            effects=[Effect("release", EffectKind.PublishRelease)],
        )
        def publish():
            return shell("./publish.sh")

        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            publish()

        tir = json.loads(ci().emit_json())
        effect_ids = tir["actions"][0].get("effects", [])
        assert len(effect_ids) == 1
        assert tir["effects"][effect_ids[0]]["name"] == "release"


class TestSecrets:
    def test_secrets_in_action(self):
        @op(outputs={}, secrets=["DEPLOY_TOKEN"])
        def deploy():
            return shell("./deploy.sh")

        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            deploy()

        tir = json.loads(ci().emit_json())
        assert "DEPLOY_TOKEN" in tir["actions"][0].get("secrets", [])


class TestConstraints:
    def test_label_constraint_in_action(self):
        @op(outputs={"image": SourceTree}, constraints=[Constraint.label("gpu")])
        def train(data: SourceTree):
            return shell("python train.py")

        @workflow
        def ci():
            actor("gpu-runner", labels=["gpu"], capabilities=["gpu"])
            data = artifact("dataset", SourceTree)
            train(data)

        tir = json.loads(ci().emit_json())
        act = next(a for a in tir["actions"] if a["name"] == "train")
        assert {"Label": "gpu"} in act.get("actor_constraints", [])


class TestPlacement:
    def test_placement_recorded(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            place(src, Placement.shared_volume("/mnt/shared"))

        tir = json.loads(ci().emit_json())
        assert len(tir.get("placements", [])) == 1
        p = tir["placements"][0]
        assert p["strategy"] == {"SharedVolume": {"path": "/mnt/shared"}}

    def test_github_artifact_placement(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            place(src, Placement.github_artifact())

        tir = json.loads(ci().emit_json())
        assert tir["placements"][0]["strategy"] == "GithubArtifact"


class TestPolicy:
    def test_max_parallel_jobs(self):
        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            max_parallel_jobs(4)
            checkout()

        tir = json.loads(ci().emit_json())
        assert tir["policies"]["max_parallel_jobs"] == 4


class TestContextManager:
    def test_graph_as_context_manager(self):
        with WorkflowGraph("manual") as g:
            actor("r", labels=["ubuntu"])
            src = checkout()
            build(src)

        tir = json.loads(g.emit_json())
        assert tir["name"] == "manual"
        assert len(tir["actions"]) == 2

    def test_context_not_active_outside(self):
        from thread._graph import current_graph
        with WorkflowGraph("temp"):
            pass
        with pytest.raises(RuntimeError):
            current_graph()


class TestMultiOutput:
    def test_multi_output_op(self):
        @op(outputs={"binary": Binary, "report": TestReport})
        def build_and_test(src: SourceTree):
            return shell("cargo test --release")

        @workflow
        def ci():
            actor("r", labels=["ubuntu"])
            src = checkout()
            result = build_and_test(src)
            sign_wheel(result.report)

        tir = json.loads(ci().emit_json())
        bat = next(a for a in tir["actions"] if a["name"] == "build_and_test")
        assert len(bat["outputs"]) == 2

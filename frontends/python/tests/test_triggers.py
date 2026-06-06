"""Tests for workflow triggers (the `on` namespace + @workflow(triggers=...))."""

import json

from ariadne import action, workflow, actor, shell, SourceTree, on


def _wf(triggers):
    @action(outputs={"src": SourceTree})
    def checkout():
        return shell("git checkout .")

    @workflow(triggers=triggers)
    def ci():
        actor("r", labels=["ubuntu-latest"])
        checkout()

    return json.loads(ci().emit_json())


class TestOnNamespace:
    def test_pull_request(self):
        assert on.pull_request().to_tir() == {"kind": "pull_request"}

    def test_push_no_branches(self):
        assert on.push().to_tir() == {"kind": "push"}

    def test_push_branches(self):
        assert on.push(branches=["main"]).to_tir() == {"kind": "push", "branches": ["main"]}

    def test_tag(self):
        assert on.tag("v*").to_tir() == {"kind": "tag", "pattern": "v*"}

    def test_schedule(self):
        assert on.schedule("0 2 * * *").to_tir() == {"kind": "schedule", "cron": "0 2 * * *"}

    def test_manual(self):
        assert on.manual().to_tir() == {"kind": "manual"}


class TestTriggersInWorkflow:
    def test_triggers_emitted_to_tir(self):
        tir = _wf([on.push(branches=["main"]), on.tag("v*"), on.manual()])
        assert tir["triggers"] == [
            {"kind": "push", "branches": ["main"]},
            {"kind": "tag", "pattern": "v*"},
            {"kind": "manual"},
        ]

    def test_no_triggers_omits_key(self):
        tir = _wf(None)
        assert "triggers" not in tir


class TestArtifactLifetime:
    def test_lifetime_duration_on_output(self):
        from ariadne import action, workflow, actor, shell, TestReport

        @action(outputs={"report": TestReport.file("junit.xml", lifetime="14d")})
        def t():
            return shell("pytest")

        @workflow
        def ci():
            actor("r", labels=["ubuntu-latest"])
            t()

        tir = json.loads(ci().emit_json())
        art = next(a for a in tir["artifacts"] if "report" in a["name"])
        assert art["lifetime"] == "14d"

    def test_lifetime_category_helper(self):
        from ariadne import ArtifactLifetime, ReleaseBundle

        decl = ReleaseBundle.file("release.tar.gz", lifetime=ArtifactLifetime.release())
        assert decl.lifetime == "release"

    def test_no_lifetime_omits_key(self):
        from ariadne import action, workflow, actor, shell, Binary

        @action(outputs={"bin": Binary.file("app")})
        def b():
            return shell("make")

        @workflow
        def ci():
            actor("r", labels=["ubuntu-latest"])
            b()

        tir = json.loads(ci().emit_json())
        art = next(a for a in tir["artifacts"] if "bin" in a["name"])
        assert "lifetime" not in art


class TestExecutionAndCoordination:
    def test_timeout_on_action(self):
        from ariadne import action, workflow, actor, shell, SourceTree

        @action(outputs={"src": SourceTree}, timeout="30m")
        def checkout():
            return shell("git checkout .")

        @workflow
        def ci():
            actor("r", labels=["ubuntu-latest"])
            checkout()

        tir = json.loads(ci().emit_json())
        call = next(c for c in tir["action_calls"] if c["name"] == "checkout")
        assert call["timeout"] == "30m"

    def test_action_coordination(self):
        from ariadne import action, workflow, actor, shell, SourceTree, coordination

        @action(outputs={"src": SourceTree}, coordination=coordination.exclusive("prod-deploy"))
        def deploy():
            return shell("./deploy.sh")

        @workflow
        def ci():
            actor("r", labels=["ubuntu-latest"])
            deploy()

        tir = json.loads(ci().emit_json())
        call = next(c for c in tir["action_calls"] if c["name"] == "deploy")
        assert call["coordination"] == {"group": "prod-deploy"}

    def test_workflow_coordination(self):
        from ariadne import action, workflow, actor, shell, SourceTree, coordination

        @action(outputs={"src": SourceTree})
        def checkout():
            return shell("git checkout .")

        @workflow(coordination=coordination.cancel_previous("release"))
        def ci():
            actor("r", labels=["ubuntu-latest"])
            checkout()

        tir = json.loads(ci().emit_json())
        assert tir["coordination"] == {"group": "release", "cancel_in_progress": True}


class TestResources:
    def test_resources_helper(self):
        from ariadne import resources

        assert resources(cpu=8, memory="32Gi", disk="100Gi").to_tir() == {
            "cpu": 8,
            "memory": "32Gi",
            "disk": "100Gi",
        }

    def test_resources_on_action(self):
        from ariadne import action, workflow, actor, shell, SourceTree, resources

        @action(outputs={"src": SourceTree}, resources=resources(cpu=8, memory="32Gi"))
        def heavy():
            return shell("make")

        @workflow
        def ci():
            actor("r", labels=["ubuntu-latest"])
            heavy()

        tir = json.loads(ci().emit_json())
        call = next(c for c in tir["action_calls"] if c["name"] == "heavy")
        assert call["resources"] == {"cpu": 8, "memory": "32Gi"}

    def test_actor_advertises_resources(self):
        from ariadne import Inventory, resources

        inv = Inventory("ci").actor(
            "big", selector=["large"], resources=resources(cpu=16, memory="64Gi")
        )
        tir = inv.to_tir()
        assert tir["actors"][0]["resources"] == {"cpu": 16, "memory": "64Gi"}

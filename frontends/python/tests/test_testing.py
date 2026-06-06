"""Tests for the loom-test authoring DSL (ariadne.testing) and in-process runner."""

from ariadne import (
    action,
    workflow,
    Inventory,
    Pipeline,
    scm,
    build,
    shell,
    Consequence,
    ConsequenceKind,
)
from ariadne.artifacts import SourceTree, Binary
from ariadne.testing import test_case, event, expect


class TestDSL:
    def test_assertion_builders_return_dicts(self):
        assert expect.consequence_gated("ship") == {"assert": "consequence_gated", "effect": "ship"}
        assert expect.max_parallel_jobs(10) == {"assert": "max_parallel_jobs", "max": 10}

    def test_event_fixtures(self):
        assert event.pull_request(fork=True) == {"pull_request": {"fork": True}}
        assert event.push("main") == {"push": {"branch": "main"}}
        assert event.tag("v1") == {"tag": {"name": "v1"}}

    def test_case_builds_a_case(self):
        @test_case(event=event.pull_request(fork=True), backend="github")
        def my_case():
            expect.consequence_gated("ship")
            expect.secret_withheld("TOKEN")

        c = my_case()
        tir = c.to_tir()
        assert tir["name"] == "my_case"
        assert tir["event"] == {"pull_request": {"fork": True}}
        assert tir["backend"] == "github"
        assert [a["assert"] for a in tir["assertions"]] == ["consequence_gated", "secret_withheld"]

    def test_explicit_name(self):
        @test_case(name="custom name")
        def f():
            expect.run_passed()

        assert f().to_tir()["name"] == "custom name"


def _release_workflow():
    inv = Inventory("ci").actor("r", selector=["ubuntu-latest"]).use("git").use("cargo")

    @action(outputs={"src": SourceTree})
    def checkout():
        return scm.checkout()

    @action(outputs={"bin": Binary.file("target/release/app")})
    def build_bin(src: SourceTree):
        return build.binary(package="app", release=True)

    @action(
        outputs={},
        consequences=[Consequence("ship", ConsequenceKind.Deployment, requires_approval=False)],
        secrets=["DEPLOY_KEY"],
    )
    def deploy(bin: Binary):
        return shell("./deploy.sh")

    @workflow(inventory=inv)
    def release():
        src = checkout()
        b = build_bin(src)
        deploy(b)

    return release()


@test_case(backend="github")
def checkout_emits_native_action():
    # Source is reacquired per job: the consumer's checkout step renders as the
    # native actions/checkout, not a download.
    expect.selected_instruction("CheckoutRepo", "github.checkout.default")


@test_case
def binary_path():
    expect.artifact_path("build_bin-bin", "target/release/app")


@test_case(event=event.pull_request(fork=True))
def fork_pr_gates_deploy():
    expect.consequence_gated("ship")
    expect.secret_withheld("DEPLOY_KEY")


@test_case(event=event.push("main"))
def push_fires_deploy():
    expect.consequence_fired("ship")
    expect.secret_spoofed("DEPLOY_KEY")


class TestRunner:
    def test_plan_level_cases_pass(self):
        results = Pipeline(_release_workflow()).run_tests(
            checkout_emits_native_action,
            binary_path,
            fork_pr_gates_deploy,
            push_fires_deploy,
            backend="github",
        )
        assert results.passed, results.report()

    def test_failing_assertion_is_reported(self):
        @test_case
        def wrong_path():
            expect.artifact_path("build_bin-bin", "nope/wrong")

        results = Pipeline(_release_workflow()).run_tests(wrong_path)
        assert not results.passed
        assert results.failures()[0].assertion

    def test_execution_level_is_skipped(self):
        @test_case
        def needs_a_run():
            expect.run_passed()

        results = Pipeline(_release_workflow()).run_tests(needs_a_run)
        assert {r.status for r in results.results} == {"skip"}
        assert results.passed  # skips do not fail

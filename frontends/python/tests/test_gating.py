"""Explicit ordering edges: `after=` gates one action on another with no data
flow (e.g. tests gated on a formatting check)."""

import json

from ariadne import workflow, action, shell, Inventory, Pipeline
from ariadne.artifacts import SourceTree, TestReport


def _wf():
    inv = Inventory("t").actor("r", selector=["ubuntu-latest"], capabilities=["linux"])

    @action(outputs={"src": SourceTree})
    def checkout():
        return shell("git checkout .")

    @action(outputs={})
    def fmt(src: SourceTree):
        return shell("fmt --check")

    @action(outputs={"report": TestReport.file("r.xml")})
    def test(src: SourceTree):
        return shell("run tests")

    @workflow(inventory=inv)
    def ci():
        s = checkout()
        gate = fmt(s)  # no outputs -> returns a CallRef
        test(s, after=[gate])

    return ci()


def test_after_edge_is_recorded_in_tir():
    tir = json.loads(_wf().emit_json())
    calls = {c["name"]: c for c in tir["action_calls"]}
    fmt_idx = next(i for i, c in enumerate(tir["action_calls"]) if c["name"] == "fmt")
    assert calls["test"].get("after") == [fmt_idx]


def test_barrier_gates_all_prior_actions():
    from ariadne import barrier

    inv = Inventory("t").actor("r", selector=["ubuntu-latest"], capabilities=["linux"])

    @action(outputs={"src": SourceTree})
    def checkout():
        return shell("git checkout .")

    @action(outputs={})
    def fmt(src: SourceTree):
        return shell("fmt")

    @action(outputs={"report": TestReport.file("r.xml")})
    def test(src: SourceTree):
        return shell("test")

    @workflow(inventory=inv)
    def ci():
        s = checkout()
        fmt(s)
        barrier()
        test(s)

    tir = json.loads(ci().emit_json())
    calls = tir["action_calls"]
    test_call = next(c for c in calls if c["name"] == "test")
    # `test` (after the barrier) must run after every call before it.
    assert sorted(test_call["after"]) == [0, 1]  # checkout, fmt


def test_after_edge_becomes_a_job_dependency():
    # Plan at -O0 (no fusion) so fmt and test stay separate jobs; the test job
    # must declare needs on the fmt job.
    yaml = Pipeline(_wf()).compile(backend="github", level=0)
    # The fmt job has no outputs but is a real job the test job depends on.
    assert "fmt" in yaml
    assert "needs" in yaml

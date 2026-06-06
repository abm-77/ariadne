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


def test_after_edge_becomes_a_job_dependency():
    # Plan at -O0 (no fusion) so fmt and test stay separate jobs; the test job
    # must declare needs on the fmt job.
    yaml = Pipeline(_wf()).compile(backend="github", level=0)
    # The fmt job has no outputs but is a real job the test job depends on.
    assert "fmt" in yaml
    assert "needs" in yaml

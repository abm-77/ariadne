"""End-to-end: the objective order + a profile drive cost-aware sibling fusion
through the frontend. Same workflow, two objective orders, different job counts.
"""

from ariadne import workflow, action, shell, objectives, Inventory, Pipeline
from ariadne.artifacts import SourceTree, Binary


def _job_count(yaml: str) -> int:
    return yaml.count("runs-on:")


def _fan_out(cost_first: bool):
    """checkout -> three distinct, independent sibling jobs on one runner."""
    inv = Inventory("t").actor("runner", selector=["ubuntu-latest"], capabilities=["linux"])

    @action(outputs={"src": SourceTree})
    def checkout():
        return shell("git checkout .")

    @action(outputs={"a": Binary.file("a.txt")})
    def fmt(src: SourceTree):
        return shell("echo fmt")

    @action(outputs={"b": Binary.file("b.txt")})
    def docs(src: SourceTree):
        return shell("echo docs")

    @action(outputs={"c": Binary.file("c.txt")})
    def cover(src: SourceTree):
        return shell("echo cover")

    @workflow(inventory=inv)
    def ci():
        if cost_first:
            objectives("dollar_cost", "critical_path")
        s = checkout()
        fmt(s)
        docs(s)
        cover(s)

    return ci()


# Runner time priced so the cost model can value the setup overhead packing saves.
PROFILE = {"runner_costs": {"ubuntu-latest": 0.01}}


def test_latency_first_keeps_siblings_parallel():
    yaml = Pipeline(_fan_out(cost_first=False)).compile(backend="github", level=3, profile=PROFILE)
    # checkout + 3 parallel siblings.
    assert _job_count(yaml) == 4


def test_cost_first_packs_siblings():
    latency = Pipeline(_fan_out(cost_first=False)).compile(backend="github", level=3, profile=PROFILE)
    cost = Pipeline(_fan_out(cost_first=True)).compile(backend="github", level=3, profile=PROFILE)
    # Same workflow; only the objective order differs. Dollars-first packs the
    # three independent siblings onto one job (checkout + one packed job).
    assert _job_count(cost) < _job_count(latency)
    assert _job_count(cost) == 2


def test_cost_first_without_profile_does_not_pack():
    # With no runner pricing, packing shows no saving the model can see, so it
    # must not trade away parallelism speculatively.
    cost = Pipeline(_fan_out(cost_first=True)).compile(backend="github", level=3)
    assert _job_count(cost) == 4

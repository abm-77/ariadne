"""Reusable CI building blocks: a shared inventory, semantic action definitions,
and composite builders that the per-workflow scripts (build-ariadne, build-python)
and the main-branch CI compose. Reuse happens at the authoring layer.

Everything is a semantic action; the runner/tool for each is selected from the
inventory, disambiguated by `impls([...])` blocks. All produced artifacts carry
a 1-day retention for now.
"""

import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "frontends", "python"))

from ariadne import (
    action,
    shell,
    Inventory,
    scm,
    build,
    test,
    fmt,
    docs,
    coverage,
    package,
    impls,
)
from ariadne.artifacts import (
    SourceTree,
    Binary,
    Wheel,
    TestReport,
    CoverageData,
    DocsSite,
    ProfileData,
)

# Keep every uploaded artifact short-lived (GitHub floors retention at 1 day).
LIFETIME = "1h"
PROFILE_LIFETIME = "1h"


def write_workflow(
    pipeline, filename: str, backend: str = "github", level: int = 2, profile=None
) -> None:
    """Compile a pipeline to a backend config and write it under .github/workflows/.
    `profile` (dict or JSON str) guides the cost model when planning."""
    yaml = pipeline.compile(backend=backend, level=level, profile=profile)
    out = os.path.join(
        os.path.dirname(__file__), "..", ".github", "workflows", filename
    )
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w") as f:
        f.write(yaml)
    print(f"wrote {out}")


def inventory() -> Inventory:
    """The execution resources and implementation technologies CI may use."""
    return (
        Inventory("ariadne-ci")
        .actor("runner", selector=["ubuntu-latest"], capabilities=["linux", "x86_64"])
        .use("git")
        .use("rust", channel="stable")
        .use("cargo")
        .use("python", version="3.12")
        .use("maturin")
        .use("ruff")
        .use("pdoc")
        .use("pytest")
    )


@action(outputs={"src": SourceTree})
def checkout():
    return scm.checkout()


@action(outputs={"loom": Binary.file("target/release/loom", lifetime=LIFETIME)})
def build_loom(src: SourceTree):
    return build.binary(src=src, package="loom", release=True)


@action(outputs={"report": TestReport.file("test-results.xml", lifetime=LIFETIME)})
def test_workspace(src: SourceTree, loom: Binary):
    return test.unit(args=["--workspace", "--", "--test-threads=4"])


@action(outputs={})
def rust_fmt(src: SourceTree):
    return fmt.check(using="cargo")


@action(outputs={"docs": DocsSite.dir("target/doc", lifetime=LIFETIME)})
def rust_docs(src: SourceTree):
    return docs.generate()


@action(outputs={"coverage": CoverageData.file("lcov.info", lifetime=LIFETIME)})
def rust_coverage(src: SourceTree):
    return coverage.measure(out="lcov.info")


@action(outputs={"wheel": Wheel.glob("dist/*.whl", lifetime=LIFETIME)})
def build_wheel(src: SourceTree):
    # Build from the frontend's pyproject (python-source + the ariadne_core
    # extension) so the wheel is the importable `ariadne` package.
    return build.python_wheel(
        src=src, dir="frontends/python", out="../../dist", release=True
    )


@action(outputs={"env": Wheel})
def install_wheel(src: SourceTree, wheel: Wheel):
    """Install the built wheel and yield the resulting environment (a marker the
    python checks consume so they run with `ariadne` importable). The marker is
    never transferred: the consumers are colocated with this install by fusion."""
    return package.install("dist/*.whl", using="pip")


@action(outputs={"report": TestReport.file("py-test-results.xml", lifetime=LIFETIME)})
def test_wheel(src: SourceTree, env: Wheel):
    return test.unit(paths=["frontends/python/tests/"], args=["-v"])


@action(outputs={})
def python_fmt(src: SourceTree):
    return fmt.check(paths=["frontends/python"], using="ruff")


@action(outputs={"docs": DocsSite.dir("frontends/python/docs", lifetime=LIFETIME)})
def python_docs(src: SourceTree, env: Wheel):
    return docs.generate(package="ariadne", out="frontends/python/docs")


@action(outputs={"coverage": CoverageData.file("py-coverage.xml", lifetime=LIFETIME)})
def python_coverage(src: SourceTree, env: Wheel):
    return coverage.measure(
        paths=["frontends/python/tests"], package="ariadne", out="py-coverage.xml"
    )


@action(
    outputs={"profile": ProfileData.file("profile.json", lifetime=PROFILE_LIFETIME)}
)
def refresh_profile(src: SourceTree):
    """Close the profile-guided loop: aggregate recent main-branch runs into a
    fresh profile.json (durations, sizes, setup/queue, runner cost) and upload
    it. Re-running the CI generators with this profile self-tunes future plans.

    This is CI tooling (the `loom profile` collector), so it uses the shell
    escape hatch; `gh` authenticates with the workflow's GITHUB_TOKEN."""
    return shell(
        "cargo build --release -p loom\n"
        "./target/release/loom profile github --workflow main.yml --runs 20 --out profile.json",
        env={"GH_TOKEN": "${{ secrets.GITHUB_TOKEN }}"},
    )

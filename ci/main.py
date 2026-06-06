import json
import os
import sys

import _lib
from _lib import (
    inventory,
    checkout,
    build_loom,
    test_workspace,
    rust_fmt,
    rust_docs,
    rust_coverage,
    build_wheel,
    install_wheel,
    test_wheel,
    python_fmt,
    python_docs,
    python_coverage,
    refresh_profile,
    commit_profile,
)
from ariadne import (
    workflow,
    Pipeline,
    on,
    impls,
    install_dependencies,
    objectives,
    barrier,
)
from ariadne.testing import test_case, expect

PROFILE_PATH = os.path.join(os.path.dirname(__file__), "..", "profile.json")

# Runner pricing so the cost model values fewer jobs; lets sibling fusion auto-
# group same-toolchain work. Real timings arrive via the committed profile.json.
PRICING = {"runner_costs": {"ubuntu-latest": 0.008 / 60}}


def load_profile():
    """The committed profile from the last collection merged over runner pricing,
    so the cost model always has costs to reason about."""
    profile = dict(PRICING)
    if os.path.exists(PROFILE_PATH):
        with open(PROFILE_PATH) as f:
            profile.update(json.load(f))
    return profile


@workflow(inventory=inventory(), triggers=[on.push(branches=["main"])])
def main_ci():
    # Provision each job's toolchain + tools; trade latency for fewer jobs so the
    # optimizer fuses same-toolchain work (Rust built once, wheel installed once).
    install_dependencies()
    objectives("dollar_cost", "critical_path")
    src = checkout()

    rust_fmt(src)
    python_fmt(src)
    barrier()

    with impls(["cargo"]):
        loom = build_loom(src)
        test_workspace(src, loom)
        rust_docs(src)
        rust_coverage(src)

    with impls(["maturin", "ruff", "pdoc", "pytest"]):
        wheel = build_wheel(src)
        env = install_wheel(src, wheel)
        test_wheel(src, env)
        python_docs(src, env)
        python_coverage(src, env)

    # Re-collect the optimizer profile from recent runs and commit it back so the
    # next generation plans with real timings (fusion colocates collect + commit).
    profile = refresh_profile(src)
    commit_profile(src, profile)


@test_case(name="docs and coverage are produced for both languages")
def artifacts_produced():
    expect.artifact_produced("rust_docs-docs")
    expect.artifact_produced("python_docs-docs")
    expect.artifact_produced("rust_coverage-coverage")
    expect.artifact_produced("python_coverage-coverage")


@test_case(name="profile is refreshed each run")
def profile_refreshed():
    expect.artifact_produced("refresh_profile-profile")


def main():
    pipeline = Pipeline(main_ci())

    if pipeline.has_errors():
        for d in pipeline.validate():
            print(f"error: {d}", file=sys.stderr)
        sys.exit(1)

    profile = load_profile()

    results = pipeline.run_tests(
        artifacts_produced,
        profile_refreshed,
        backend="github",
        level=3,
        profile=profile,
    )
    if not results.passed:
        print("loom tests failed:", file=sys.stderr)
        print(results.report(), file=sys.stderr)
        sys.exit(1)

    _lib.write_workflow(pipeline, "main.yml", level=3, profile=profile)


if __name__ == "__main__":
    main()

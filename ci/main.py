"""Main-branch CI: runs on push to main. Formats, builds, tests, docs, and
covers BOTH the Rust engine and the Python frontend, then uploads the artifacts
(retention 1d for now). Composes the reusable builders from `_lib`.

It also refreshes the optimizer profile from recent runs (the profile-guided
loop): if a committed profile.json exists, this run is planned with it; and the
refresh_profile job re-collects a fresh one for the next generation to use."""

import json
import os
import sys

import _lib
from _lib import (
    inventory, checkout,
    ariadne_build_and_test, python_build_and_test,
    rust_fmt, rust_docs, rust_coverage,
    python_fmt, python_docs, python_coverage,
    refresh_profile,
)
from ariadne import workflow, Pipeline, on, impls
from ariadne.testing import test_case, expect

PROFILE_PATH = os.path.join(os.path.dirname(__file__), "..", "profile.json")


def load_profile():
    """The committed profile from the last collection, if any. Planning with it
    makes the cost model (and thus packing decisions) reflect real timings."""
    if os.path.exists(PROFILE_PATH):
        with open(PROFILE_PATH) as f:
            return json.load(f)
    return None


@workflow(inventory=inventory(), triggers=[on.push(branches=["main"])])
def main_ci():
    src = checkout()

    with impls(["cargo"]):
        rust_fmt(src)
        ariadne_build_and_test(src)
        rust_docs(src)
        rust_coverage(src)

    with impls(["maturin", "ruff", "pdoc", "pytest"]):
        python_fmt(src)
        wheel = python_build_and_test(src)
        python_docs(src, wheel)
        python_coverage(src, wheel)

    # Re-collect the optimizer profile from recent runs and upload it.
    refresh_profile(src)


@test_case(name="runs only on push to main")
def push_main_only():
    # plan-level: docs and coverage artifacts are produced for both languages.
    expect.artifact_produced("rust_docs/docs")
    expect.artifact_produced("python_docs/docs")
    expect.artifact_produced("rust_coverage/coverage")
    expect.artifact_produced("python_coverage/coverage")


@test_case(name="profile is refreshed each run")
def profile_refreshed():
    expect.artifact_produced("refresh_profile/profile")


def main():
    pipeline = Pipeline(main_ci())

    if pipeline.has_errors():
        for d in pipeline.validate():
            print(f"error: {d}", file=sys.stderr)
        sys.exit(1)

    profile = load_profile()

    # Main CI ships at -O3: fusion collapses producer -> sole-consumer chains
    # onto one runner, dropping their upload/download. Plan with the collected
    # profile (if any) and test the level we emit.
    results = pipeline.run_tests(push_main_only, profile_refreshed, backend="github", level=3, profile=profile)
    if not results.passed:
        print("loom tests failed:", file=sys.stderr)
        print(results.report(), file=sys.stderr)
        sys.exit(1)

    _lib.write_workflow(pipeline, "main.yml", level=3, profile=profile)


if __name__ == "__main__":
    main()

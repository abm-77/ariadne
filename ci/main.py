"""Main-branch CI: runs on push to main. Formats, builds, tests, docs, and
covers BOTH the Rust engine and the Python frontend, then uploads the artifacts.

Architecture is expressed as intent, not jobs:
  * Testing is gated on formatting via explicit `after=` edges (fmt -> the rest).
  * Each job provisions its own toolchain + tools (install_dependencies).
  * The cost model + sibling fusion auto-group independent same-toolchain actions
    onto one runner (shared build), so Rust compiles once and the python checks
    share one wheel install. This needs a cost-leaning objective + runner pricing.

It also refreshes the optimizer profile from recent runs (the profile-guided
loop): a committed profile.json plans this run; refresh_profile re-collects one.
"""

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
    build_core_ext,
    build_wheel,
    install_wheel,
    test_wheel,
    python_fmt,
    python_docs,
    python_coverage,
    refresh_profile,
)
from ariadne import workflow, Pipeline, on, impls, install_dependencies, objectives
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

    # Rust: formatting gates build/test/docs/coverage; fusion groups them.
    with impls(["cargo"]):
        rfmt = rust_fmt(src)
        loom = build_loom(src, after=[rfmt])
        test_workspace(src, loom)
        rust_docs(src, after=[rfmt])
        rust_coverage(src, after=[rfmt])

    # Python: formatting gates everything; the wheel is built, installed, then
    # exercised. Fusion colocates install with the checks so the package imports.
    with impls(["maturin", "ruff", "pdoc", "pytest"]):
        pfmt = python_fmt(src)
        ext = build_core_ext(src, after=[pfmt])
        wheel = build_wheel(src, ext)
        inst = install_wheel(src, wheel)
        test_wheel(src, wheel, after=[inst])
        python_docs(src, wheel, after=[inst])
        python_coverage(src, wheel, after=[inst])

    # Re-collect the optimizer profile from recent runs and upload it.
    refresh_profile(src)


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

    # Ships at -O3 (fusion enabled); plan with the profile and test that level.
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

"""Declared tool dependencies + install-on-start. Implementations declare the
tools they need; by default the environment provides them, but a workflow can
opt into installing them on job start. The install commands live in a shared
plan-level table, so tools used by many jobs are stored once.
"""

from ariadne import workflow, action, Inventory, build, test, install_dependencies, Pipeline
from ariadne.artifacts import SourceTree, Wheel, TestReport


def _wf(install: bool):
    inv = (
        Inventory("t")
        .actor("runner", selector=["ubuntu-latest"], capabilities=["linux"])
        .use("python", version="3.12")
        .use("maturin")
        .use("pytest")
    )

    @action(outputs={"src": SourceTree})
    def checkout():
        from ariadne import shell

        return shell("git checkout .")

    @action(outputs={"wheel": Wheel.glob("dist/*.whl")})
    def build_wheel(src: SourceTree):
        return build.python_wheel(src=src, package="pkg", out="dist")

    @action(outputs={"report": TestReport.file("r.xml")})
    def run_tests(src: SourceTree, wheel: Wheel):
        return test.unit(paths=["tests/"])

    @workflow(inventory=inv)
    def ci():
        if install:
            install_dependencies()
        s = checkout()
        w = build_wheel(s)
        run_tests(s, w)

    return ci()


def test_no_install_by_default():
    yaml = Pipeline(_wf(install=False)).compile(backend="github", level=0)
    assert "Install dependencies" not in yaml
    assert "pip install maturin" not in yaml
    assert "actions/setup-python" not in yaml


def test_install_on_start_provisions_declared_tools():
    yaml = Pipeline(_wf(install=True)).compile(backend="github", level=0)
    assert "Install dependencies" in yaml
    # Each job installs the tool its action declared.
    assert "pip install maturin" in yaml  # build_wheel -> maturin
    assert "pip install pytest" in yaml  # run_tests -> pytest


def test_install_on_start_provisions_toolchains_from_inventory():
    yaml = Pipeline(_wf(install=True)).compile(backend="github", level=0)
    # The Python toolchain (used by maturin/pytest) is set up per job, at the
    # version the inventory declared.
    assert "actions/setup-python@v5" in yaml
    assert "python-version: '3.12'" in yaml

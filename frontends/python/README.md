# ariadne (Python frontend)

`ariadne` is the Python authoring frontend for [Ariadne](../../README.md), a
CI/CD workflow planning compiler. You declare workflows as semantic intent in
Python; Ariadne validates them, plans execution, optimizes artifact movement and
runner placement, and emits backend config such as GitHub Actions YAML. The
package embeds the Rust engine as a native extension (`ariadne.ariadne_core`),
so planning runs in process with no separate service.

> You describe what the pipeline is made of. Ariadne decides how to run it.

## Install

For development, from this directory:

```sh
maturin develop          # builds the native extension and installs the package editable
pytest tests/
```

This requires a Rust toolchain and [maturin](https://www.maturin.rs/). Build the
extension from this directory (its pyproject configures the package layout and
the `ariadne.ariadne_core` module); building from `crates/ariadne-py` produces a
bare extension that the `ariadne` package will not pick up.

## Concepts

A workflow is a graph of **actions** that consume and produce typed
**artifacts**. An action names a **semantic operation** (`build.binary`,
`scm.checkout`, `test.unit`), never a tool command. An **inventory** declares the
execution resources (actors) and the implementations available (cargo, maturin,
pytest, and so on); Ariadne specifies each action to an implementation and lowers
it to a backend step. Actions may declare **consequences** (Deployment, GitWrite,
PublishRelease, ...), which drive permissions, gating, and safety analysis.

You never write `needs:`, upload/download steps, caches, or matrices. Data flow
between actions is the dependency graph, and Ariadne derives the rest.

## A first workflow

```python
from ariadne import action, workflow, Inventory, Pipeline, scm, build, test, on
from ariadne.artifacts import SourceTree, Binary, TestReport


def inventory() -> Inventory:
    return (
        Inventory("ci")
        .actor("runner", selector=["ubuntu-latest"], capabilities=["linux"])
        .use("git")
        .use("cargo")
    )


@action(outputs={"src": SourceTree})
def checkout():
    return scm.checkout()


@action(outputs={"bin": Binary.file("target/release/app")})
def build_app(src: SourceTree):
    return build.binary(package="app", release=True)


@action(outputs={"report": TestReport.file("results.xml")})
def run_tests(src: SourceTree, bin: Binary):
    return test.unit(args=["--workspace"])


@workflow(inventory=inventory(), triggers=[on.push(branches=["main"])])
def ci():
    src = checkout()
    binary = build_app(src)
    run_tests(src, binary)


if __name__ == "__main__":
    pipeline = Pipeline(ci())
    if pipeline.has_errors():
        raise SystemExit("\n".join(pipeline.validate()))
    print(pipeline.compile(backend="github", level=2))
```

Passing artifact handles between actions (`src`, `binary`) is what wires the
graph. Ariadne derives job dependencies, artifact transfer, and runner placement
from it.

## Semantic action namespaces

`scm`, `build`, `test`, `fmt`, `docs`, `coverage`, `scan`, `sign`, `package`,
`forge`. Each call records intent; the inventory decides the tool. For example
`test.unit(...)` becomes `cargo test` under `.use("cargo")` or `pytest` under
`.use("pytest")`. Pin the implementation for one call with `using=`, or bias a
block of calls with `impls([...])`:

```python
with impls(["cargo", "pytest"]):
    rust_tests(src)     # specifies to cargo
    python_tests(src)   # specifies to pytest
```

For tool-specific or backend-native work, `shell(...)` and
`container(image).exec(...)` are typed escape hatches. Declare their inputs,
outputs, consequences, and secrets; the compiler trusts only what is declared.

## Compiling and inspecting

`Pipeline` wraps the engine:

```python
p = Pipeline(ci())
p.validate()                                                 # structured diagnostics
p.has_errors()                                               # bool
yaml = p.compile(backend="github", level=3, profile=prof)    # validate + plan + optimize + emit
tir  = p.to_tir_json()                                       # canonical Thread IR
```

`level` is the optimization level (0 to 3). `profile` (a dict or JSON string of
observed run durations and artifact sizes) guides the cost model without changing
semantics.

## Testing workflows

Workflows are testable without executing them. Plan-level assertions check the
plan that would be produced under a given event:

```python
from ariadne.testing import test_case, event, expect


@test_case(event=event.pull_request(fork=True), backend="github")
def fork_pr_does_not_deploy():
    expect.consequence_gated("deploy")


@test_case(event=event.tag("v*"))
def tag_publishes():
    expect.consequence_fired("publish")


results = Pipeline(release()).run_tests(fork_pr_does_not_deploy, tag_publishes)
assert results.passed, results.report()
```

`expect` covers artifacts produced, consequences fired or gated, secrets spoofed
or withheld, transfers used, policy limits (`max_parallel_jobs`,
`max_concurrent_deployments`), the selected instruction per op, and warnings.
Event fixtures (`event.push`, `event.pull_request(fork=True)`, `event.tag`) drive
consequence gating and secret availability. When a case has no plan-level
assertions and a runtime is available, `loom test` can execute the workflow once
in Podman with consequences mocked and secrets spoofed.

## Relationship to Loom and Thread IR

The frontend produces **Thread IR**, the stable interchange the engine plans
over. `Pipeline.compile` runs the engine in process and returns backend config
directly, so Loom is not required. When you do want the CLI (`loom check`,
`loom explain`, `loom plan`, `loom test`), write the IR with
`Pipeline.to_tir_json()` and point Loom at the `.json` file. See the
[top-level README](../../README.md) and [DESIGN.md](../../DESIGN.md) for the
engine.

# Ariadne

## A Compiler for CI/CD Workflow Planning

## Elevator Pitch

Modern CI/CD systems force engineers to manually plan execution: jobs, dependencies, artifacts, caches, runners, matrices, uploads, and downloads.

Ariadne separates workflow semantics from execution strategy.

Users describe artifacts, actions, effects, constraints, and policies. Ariadne generates a correct execution plan, optimizes artifact movement and runner placement, supports local execution, explains decisions, and emits standard CI configurations.

Loom is the reference frontend and CLI.

---

## Core Thesis

CI/CD workflows are planning problems.

Existing CI systems expose execution mechanics:

- jobs
- needs
- matrices
- artifacts
- caches
- runners
- upload/download steps

Ariadne exposes workflow semantics and derives execution.

```text
User intent
    ↓
Thread IR
    ↓
Ariadne planner
    ↓
Execution plan
    ↓
GitHub Actions / GitLab CI / Local containers
```

---

## Non-Goals

Ariadne is not:

- a CI provider
- Kubernetes
- Slurm
- Airflow
- a cluster scheduler
- a general DAG engine

Ariadne compiles onto existing CI/CD infrastructure.

---

## Project Components

### Ariadne

The core planning engine.

Ariadne owns:

- Thread IR validation
- type/effect analysis
- placement planning
- actor selection
- optimization
- diagnostics
- profile-guided planning
- backend emission

### Loom

The reference frontend and CLI.

Loom owns:

- workflow authoring
- Python APIs
- developer ergonomics
- local commands
- Thread IR generation

Ariadne can be used without Loom.

---

## Thread IR

Thread IR is Ariadne's canonical semantic representation.

All frontends produce Thread IR. Ariadne consumes Thread IR.

Thread IR is both:

- an in-memory model
- a serialized interchange format

Thread IR is useful for:

- testing
- debugging
- bug reports
- agent workflows
- frontend interoperability
- snapshotting

Normal users are not expected to write Thread IR manually.

---

## Core Semantic Model

Thread IR has five first-class concepts:

- Artifact
- Action
- Effect
- Placement
- Actor

---

## Artifact

Artifacts are logical workflow data.

Examples:

- SourceTree
- Wheel
- Binary
- ContainerImage
- SBOM
- Signature
- ReleaseBundle
- CoverageReport
- TestReport
- Dataset
- Model

Artifacts are:

- typed
- immutable
- logical

Artifacts are not merely files. They may have multiple physical representations.

---

## Action

Actions are computation.

Examples:

- checkout
- build
- test
- scan
- package
- sign
- release
- deploy

Actions consume artifacts, produce artifacts, and may emit effects.

---

## Effect

Effects are external mutation or privileged interaction.

Examples:

- Network
- SecretAccess
- GitWrite
- PublishRelease
- Deployment
- CommentOnPR

Effects enable:

- approval gates
- dry runs
- safety analysis
- optimization barriers
- policy enforcement

---

## Placement

Placement is the physical realization of an artifact.

Examples:

- GitHub artifact
- GitLab artifact
- object storage
- container registry
- shared volume
- persistent cache
- OCI layer
- filesystem path

Placement answers:

- where does data live?
- how is data accessed?
- what does access cost?

Placement is a first-class planning concept.

---

## Actor

Actors are execution resources.

Examples:

- github-ubuntu
- github-macos
- self-hosted-linux
- self-hosted-arm64
- self-hosted-gpu
- local-docker

Actors expose capabilities:

- linux
- macos
- arm64
- gpu
- docker
- shared storage
- cache volume access

Ariadne assigns actions to actors. It does not provision actors.

---

## Graph Model

Thread IR is a typed graph.

Relationships:

```text
Action produces Artifact
Action consumes Artifact
Action emits Effect
Artifact has Placement
Action executes on Actor
```

Execution is derived from artifact flow.

---

## Policies and Constraints

Backend capabilities define what is possible.

User policies define what is allowed.

Examples:

- max parallel jobs
- max GPU jobs
- reserve runner capacity
- max artifact transfer
- max deployment concurrency
- cost limits
- network limits

Policies are part of Thread IR.

Frontends construct policies. Ariadne enforces them.

---

## Backend Capability Model

Backends advertise capabilities.

Examples:

- GitHub Actions
- GitLab CI
- Local Docker
- Namespace runners
- self-hosted runners

Capabilities include:

- artifact storage
- cache support
- runner types
- shared storage
- secrets
- approvals
- matrix support
- cross-job mounts

Example:

```text
GitHub hosted:
  cross-job mounts = false
  artifacts = true

Self-hosted:
  cross-job mounts = possible
  shared storage = possible
```

The same Thread IR can compile differently depending on backend capabilities.

---

## Correctness Invariant

Ariadne must always generate a correct plan if one exists.

Optimization is optional. Correctness is mandatory.

Planning pipeline:

```text
Thread IR
    ↓
Validation
    ↓
Analysis
    ↓
Correct Plan
    ↓
Optimization
    ↓
Execution Plan
```

Fallback behavior is required.

Example:

```text
preferred: mount
fallback: upload/download
```

A workflow must not fail solely because an optimization is unavailable.

---

## Placement Planning

Traditional CI requires users to manually write:

- upload artifact
- download artifact
- cache restore
- cache save

Ariadne treats this as a planning problem.

Example:

```text
build_model
    ↓
Model
    ↓
50 consumers
```

Naive plan:

```text
upload once
download 50 times
```

Optimized plan when supported:

```text
place on shared volume
colocate consumers
mount read-only
```

If unavailable:

```text
fall back to upload/download
warn about cost
```

---

## Access Modes

Artifacts may be accessed by:

- Copy
- MountReadOnly
- MountReadWrite
- Stream
- SameHostPath
- OCILayer

The planner selects an access mode based on:

- artifact type
- artifact size
- consumer count
- actor capabilities
- backend capabilities
- policy constraints
- profile data

---

## Optimization Goals

Ariadne optimizes:

- data movement
- execution latency
- compute cost
- artifact materialization
- runner utilization
- workflow complexity

while preserving semantics.

---

## Optimization Passes

### Deduplication

Reuse identical pure actions.

### Hoisting

Materialize shared dependencies once.

### Fusion

Combine cheap adjacent actions to avoid unnecessary artifact boundaries.

### Parallelization

Run independent work concurrently within policy limits.

### Placement Optimization

Prefer mount, stream, colocation, or cache placement over repeated copy/download when legal.

### Actor Optimization

Select runners with suitable capabilities and lower cost.

### Effect-Aware Optimization

Never reorder unsafe effects.

Deployments, releases, signing, and external mutations preserve ordering guarantees.

---

## Profile-Guided Optimization

Ariadne can collect profiles from previous runs.

Metrics:

- artifact sizes
- upload times
- download times
- cache hit rates
- queue times
- action durations
- failure rates
- runner costs

Profiles improve future plans.

Example:

```text
Artifact: model
Observed size: 20GB
Consumers: 48
Previous transfer: 960GB

New plan:
  colocate consumers
  mount read-only

Expected transfer:
  20GB
```

Profile data may improve optimization but must never change workflow semantics.

---

## Explainability

Ariadne must explain plans.

Example:

```bash
loom explain
```

Output:

```text
Artifact: wheel

Decision:
  materialize once

Reason:
  consumed by 8 downstream actions
```

Example:

```text
Artifact: model

Decision:
  fallback to copy

Reason:
  selected backend does not support cross-job mounts
```

Explainability is core to user trust.

---

## Local Execution

Ariadne supports local execution through Loom.

Example:

```bash
loom run
```

Initial container runtimes:

- Docker
- Podman

The runtime is abstracted.

Benefits:

- debugging
- fast iteration
- reproducibility
- testing CI before CI

---

## Testing CI Itself

Local dry run enables a major feature: CI tests for CI itself.

Ariadne should allow workflows to be validated, planned, simulated, and asserted before being pushed to remote CI.

---

## Workflow as Testable Program

Workflows should be testable like application code.

Commands:

```bash
loom check
loom plan
loom test
loom emit --check
loom simulate
```

---

## `loom check`

Validates frontend output and Thread IR.

Checks:

- graph well-formedness
- artifact type correctness
- missing producers
- illegal cycles
- effect declarations
- policy validity
- backend compatibility

---

## `loom plan`

Produces an execution plan without running it.

Useful for:

- reviewing graph shape
- checking effects
- checking artifact movement
- checking runner assignment
- estimating cost

---

## `loom test`

Runs workflow tests.

Possible checks:

- validate Thread IR
- plan for target backend
- assert expected artifacts
- assert expected effects
- assert policy compliance
- assert emitted CI is up to date
- assert unsafe effects are gated

---

## Workflow Unit Tests

Frontends can expose test APIs.

Example:

```python
def test_release_plan():
    wf = release_workflow()

    plan = ariadne.plan(
        wf,
        backend="github",
        mode="dry_run",
    )

    assert plan.has_artifact("wheel")
    assert plan.has_effect("publish_release")
    assert plan.effect("publish_release").requires_approval()
    assert plan.max_parallel_jobs <= 16
```

These tests do not run expensive jobs. They test workflow structure and semantics.

---

## Local Integration Tests

Ariadne can locally execute non-effectful parts of a workflow.

Example:

```bash
loom run release --local --dry-effects
```

Effectful actions are mocked.

Examples:

```text
publish release -> mocked
deploy prod -> mocked
push image -> local registry or mocked
comment on PR -> mocked
```

Build, test, package, and scan actions can run in containers.

---

## Event Fixture Testing

Ariadne should simulate CI event contexts.

Examples:

```bash
loom test --event pull_request
loom test --event push-main
loom test --event tag-release
```

Fixtures model:

- branch
- tag
- commit SHA
- pull request origin
- trusted/untrusted context
- fork status

Example assertions:

```python
assert plan.on_event("pull_request").does_not_have_effect("deploy_prod")
assert plan.on_event("tag-release").has_effect("publish_release")
assert plan.on_event("fork-pr").does_not_access_secret("PROD_TOKEN")
```

This prevents dangerous CI mistakes.

---

## Policy Tests

Examples:

```python
assert plan.max_parallel_jobs <= 20
assert plan.no_effects_on_untrusted_pull_request()
assert plan.deployments_require_approval()
assert plan.no_secret_access_in_fork_context()
assert plan.max_gpu_jobs <= 4
```

This is especially useful for enterprise release pipelines.

---

## Plan Shape Tests

Users can assert expected planner behavior.

Example:

```python
def test_large_artifact_uses_mount_when_available():
    plan = ariadne.plan(
        model_eval_workflow(),
        backend=fake_backend(
            supports_mounts=True,
            supports_colocation=True,
        ),
    )

    assert plan.artifact("model").access_mode == "mount_read_only"
```

Fallback test:

```python
def test_large_artifact_falls_back_to_copy_on_github():
    plan = ariadne.plan(
        model_eval_workflow(),
        backend="github-hosted",
    )

    assert plan.artifact("model").access_mode == "copy"
    assert plan.has_warning("mount unavailable")
```

---

## Golden Tests

Generated backend output should be snapshot-testable.

Example:

```bash
loom emit github > .github/workflows/release.yml
git diff --exit-code
```

or:

```text
tests/snapshots/release.github.yml
```

This catches accidental workflow changes.

---

## Testing CI in CI

A repository can use Ariadne to validate its own CI.

Example GitHub workflow:

```yaml
name: Validate CI

on:
  pull_request:

jobs:
  validate-ci:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: loom check
      - run: loom plan --backend github
      - run: loom emit github --check
      - run: loom test --local --dry-effects
```

This lets CI validate future CI changes before they land.

---

## Agent-Friendly Design

Agents are important users.

Preferred flow:

```text
User intent
    ↓
Agent
    ↓
Loom / Thread IR
    ↓
Ariadne
    ↓
Structured diagnostics
    ↓
Agent repair loop
    ↓
Execution plan
```

Compiler features that help agents:

- typed artifacts
- structured diagnostics
- serialized Thread IR
- explainable plans
- local simulation
- plan assertions
- safe effect mocks

Ariadne makes LLM-generated CI safer by moving correctness and planning into the compiler.

---

## Bindings

Ariadne supports multiple use modes:

- CLI
- native bindings
- serialized Thread IR

Bindings should expose:

- validate
- plan
- emit
- explain
- test

Bindings should not expose unstable compiler internals.

---

## Rust Architecture

Ariadne is implemented in Rust.

Reasons:

- performance
- safety
- portability
- good CLI ecosystem
- easy distribution
- strong testing support

Ariadne is not built on LLVM or MLIR.

The hard problem is domain planning, not machine-code lowering.

---

## Workspace Layout

```text
crates/

thread-ir
thread-ir-serde

diagnostics
validate
analysis
planner
profile

backends
backend-github
backend-local

ariadne

loom
```

Dependency flow:

```text
thread-ir
    ↑
diagnostics
    ↑
validate
    ↑
analysis
    ↑
planner
    ↑
backends
    ↑
ariadne
    ↑
loom
```

---

## Testing Ariadne Itself

Ariadne should use golden tests and fixture workflows.

Test categories:

- validation tests
- type-checking tests
- effect-safety tests
- placement fallback tests
- placement optimization tests
- policy constraint tests
- backend emission tests
- local execution tests
- profile-guided optimization tests

Important invariant tests:

- backend lacks mounts -> copy fallback works
- deployment effect -> never reordered before tests
- policy max_parallel_jobs = 10 -> plan never exceeds 10
- large artifact + mount backend -> mount selected
- large artifact + GitHub hosted -> copy selected with warning

---

## Initial Milestones

### Milestone 1

- Thread IR
- validator
- naive planner
- GitHub emitter
- basic Loom CLI

### Milestone 2

- local Docker/Podman backend
- dry-effect execution
- plan explain
- golden tests

### Milestone 3

- placement-aware planner
- actor-aware scheduling
- policy constraints
- plan assertions

### Milestone 4

- profile collection
- profile-guided optimization
- agent repair loop
- additional frontends

---

## Final Thesis

Ariadne treats CI/CD as a planning problem, not a YAML authoring problem.

Loom is the reference way to author workflows.

Thread IR is the semantic graph.

Ariadne validates, plans, optimizes, explains, tests, and emits executable CI.

The goal is not a better workflow language.

The goal is a compiler that understands how CI/CD workflows should execute.

# Ariadne

## A Compiler for CI/CD Workflow Planning

## Elevator Pitch

Modern CI/CD systems force engineers to manually plan execution: jobs, dependencies, artifacts, caches, runners, matrices, uploads, and downloads.

Ariadne separates workflow semantics from execution strategy.

Users describe artifacts, actions, effects, constraints, and policies. Ariadne generates a correct execution plan, optimizes artifact movement and runner placement, supports local execution, explains decisions, and emits standard CI configurations.

The Python package `ariadne` is the reference authoring frontend; Loom is the CLI over Thread IR.

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

The core planning engine, and the Python authoring frontend (package `ariadne`).

Ariadne owns:

- Thread IR and its validation
- the Inventory (actors, placements, implementations)
- action and implementation definitions
- lowering definitions (implementation selection)
- type/effect analysis
- placement planning and actor selection
- optimization and profile-guided planning
- instruction selection and backend emission
- diagnostics

### Loom

The CLI over Thread IR. Loom consumes the Thread IR that Ariadne produces and exposes the
developer commands:

- `loom check`
- `loom plan`
- `loom test`
- `loom explain`
- `loom docs`

Lowering and selection are part of Ariadne's planning model, not Loom's command model. Ariadne
can be used without Loom (any frontend that emits Thread IR works).

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

Actions are authored as semantic intent. Actors and Placements, together with the available
implementation technologies, are declared in the Inventory (see below). Execution concerns
(triggers, coordination, timeouts, resource requirements, artifact lifetimes) are modeled
explicitly too; see Execution Semantics.

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

Actions are computation. They consume artifacts, produce artifacts, and may emit effects.

Actions are authored as **semantic intent**, never as tool invocations. A workflow says
*build a Python wheel*, not *run maturin*. The frontend exposes semantic namespaces:

```text
scm.checkout(...)

build.binary(...)
build.library(...)
build.python_wheel(...)
build.container_image(...)
build.docs(...)

test.unit(...)
test.integration(...)

scan.sbom(...)
scan.vulnerability(...)

sign.artifact(...)

package.publish(...)

forge.github(...)
```

Each call records a semantic operation in Thread IR (an `Implementation::Semantic { op, args }`)
with no command baked in. Which concrete tool realizes it is decided later, by Ariadne, from
the inventory (see Inventory and Selection Model below).

An escape hatch remains for tool-specific or backend-native work: `container(image).exec(...)`
and `shell(...)` declare an explicit implementation directly. The compiler only trusts what the
action declares (inputs, outputs, effects).

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

Actors are declared in the **Inventory** (below), not inline in the workflow body.

---

## Inventory

The Inventory describes the resources and technologies available to a workflow. It is
embedded in Thread IR so planning is self-contained, and it owns:

- **Actors** - what can execute work.
- **Placement providers** - where artifacts can live and how they can be accessed.
- **Implementations** - which technologies are available to realize semantic actions.

```python
inventory = (
    Inventory("ci")
    .actor("runner", selector=["ubuntu-latest"], capabilities=["linux", "x86_64"])
    .placement("cache", kind="cache_volume", access_modes=["mount_ro", "mount_rw"])
    .use("git")
    .use("cargo")
    .use("maturin")
    .prefer("buildkit")
    .deny("docker")
)
```

An **Implementation** is an available realization technology (git, cargo, maturin, uv, pip,
cmake, buildkit, docker, podman, gh, syft, cosign, twine, ...). It is not a workflow action.
The inventory declares which ones exist; `prefer` biases selection toward one; `deny` excludes
one. The inventory never names lowerings directly.

Design split:

```text
Workflow author   chooses semantic actions
Inventory author  chooses available implementations
Lowering author   teaches Ariadne how an implementation realizes an action
```

---

## Selection Model: from `dialect.action` to a native step

Every operation in a plan has one uniform identity: a `dialect.action` (the *logical op*),
specified to an implementation as `dialect.action.impl` (the *specified op*), then lowered to a
backend physical step. Two selection phases drive this, and they are the same pattern -
*capability-gated, ranked, explainable rule resolution* over a registry - running on one shared
engine (`src/select.rs`): `Capability`, `Stability`, a `Candidate` trait, a generic `Registry<C>`
(storage + by-key index), and `resolve(candidates, available, priority)` (filter to candidates
whose required capabilities are all available, then pick the best by `(priority, stability)`,
ties broken by registration order).

```text
Logical op        dialect.action            e.g. build.binary, scm.checkout, ci.artifact.upload
    ↓   implementation selection  (plan-time, backend-agnostic)   src/lowering/
Specified op      dialect.action.impl       e.g. build.binary.cargo, scm.checkout.git
    ↓   instruction selection     (emit-time, backend-aware)      src/backends/<b>/instructions.rs
Native step       run: / uses:
```

Dialects: `scm` / `build` / `test` / `fmt` / `docs` / `coverage` / `scan` / `sign` / `package` /
`forge` are user-domain; **`ci` is the orchestration dialect** (`ci.artifact.upload` /
`download` / `transfer`, `ci.cache.restore` / `save`, `ci.approval`). The raw `shell()` escape
hatch is `shell.run`. There is no special-cased op kind: build steps and orchestration steps go
through the same selection.

### Implementation selection (lowering)

A `LoweringDef { id, action, implementation, requires, stability, build }` teaches Ariadne how
one implementation realizes one semantic action; its `build` produces a structured, inspectable
`LoweringBody` (no DSL), which flattens to a `Specification { args, fallback }` - native-step
args (usually empty) plus a shell command any backend can run. Lowerings live in an extensible
`Registry` (`src/lowering/`, one pack module per class). Built-in packs register defaults; callers
and future distributable packs may register more.

Selection consults the inventory: `deny` excludes; `prefer` then `use` then undeclared-default
sets the rank (so a silent inventory still yields a working default, preserving correctness). The
same `test.unit()` becomes `test.unit.cargo` under `use("cargo")` and `test.unit.pytest` under
`use("pytest")`. Same intent, different specified op, decided here.

### The logical op (`LogicalOp`)

The planner emits `LogicalOp`s. Identity is uniform - every op answers `action()` and
`implementation()` - but payloads stay typed so the optimizer's structural rewrites remain
exhaustive (and `AccessMode` stays an enum). The user-domain compute carrier is
`SemanticOp { action, implementation, label, args, fallback, env }` (build / test / fmt /
`scm.checkout` / `package.publish` / ...); the `ci.*` orchestration primitives are typed variants
(`UploadArtifact`, `DownloadArtifact`, `TransferArtifact`, cache, approval), and `RunShell` is the
escape hatch. A transfer's `implementation()` is its access mode (`copy`, `mount-ro`, ...). The
plan never contains a backend-specific step.

### Instruction selection (emission)

Each backend owns a `Catalogue` of `Instruction`s, each matching on `(action, implementation)`.
The `Selector` resolves a `LogicalOp` against it (capability-gated, cheapest wins) and renders the
native step: `scm.checkout` becomes `uses: actions/checkout@v4` on GitHub or `git checkout .` on
local; `ci.artifact.upload` becomes `actions/upload-artifact@v4`; `build.binary.cargo` becomes
`run: cargo build ...`.

### Native steps and the portability invariant

Native-with-shell-fallback is the universal model. Every `SemanticOp` carries a shell `fallback`
that *any* backend can run; the backend catalogue may *upgrade* it to a native step when the
inventory permits (inventory implementations surface as `impl.<id>` capabilities; the upgrade
entry requires that capability). A backend without a concrete instruction for an op falls through
to its `AnySemantic` fallback, which runs the shell command. So `scm.checkout` upgrades to
`actions/checkout@v4`, `package.publish` to `pypa/gh-action-pypi-publish` (gated on the inventory
capability), and `build.binary.cargo` - which has no native action - simply runs `cargo build`.
This keeps the plan portable and a legal plan always available, exactly as the correctness
invariant requires.

The two containers are the same type: `lowering::Registry = select::Registry<LoweringDef>` and
`backends::Catalogue = select::Registry<Instruction>`. The phases differ only in what
capabilities are in scope and what a rule produces. One model, run twice over a growing
capability set.

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

## Execution Semantics

Several execution concerns are workflow semantics, modeled explicitly in Thread
IR rather than buried in backend configuration.

### Triggers

How a workflow is entered (`Workflow.triggers`). A trigger controls workflow
entry; it is distinct from a condition (which controls execution after entry)
and from the `EventContext` the planner uses to gate consequences. Types:
pull_request, push (optional branches), tag (pattern), schedule (cron), manual.
Authored with the `on` namespace; the GitHub backend builds the `on:` block
(push branches/tags, `schedule:`, `workflow_dispatch:`) from them, defaulting to
push+PR when none are declared.

### Coordination

Concurrency control (`Coordination { group, cancel_in_progress }`), at the
workflow level (`Workflow.coordination`) and the action level
(`ActionCall.coordination`). Exclusive means one run in the group at a time
(queue); cancel-previous cancels an in-progress run. Lowers to backend-native
mechanisms: GitHub `concurrency:` (top-level and per-job), GitLab resource
groups, local locks.

### Timeouts

Maximum execution duration per action (`ActionCall.timeout`, e.g. "30m"). A
backend-independent execution requirement; GitHub emits `timeout-minutes`.
Retries are intentionally NOT modeled: a failed job is re-run through whatever
frontend the user already has, and retrying effectful actions is unsafe.

### Resource Requirements

What an action needs to execute (`ActionCall.resources`: cpu, memory, disk,
gpu) and what an actor advertises (`Actor.resources`). Resources participate in
*actor selection*: an action is assignable to an actor only if the actor
satisfies every requirement (memory/disk compared as byte sizes). Validation
rejects a workflow whose action requires resources no actor can satisfy.
Resources are not emitted directly; they constrain which actor, and therefore
which runner, is chosen.

### Artifact Lifetime

How long an artifact is retained (`Artifact.lifetime`): a category (ephemeral,
workflow, release, permanent) or a duration ("14d", "12h"). This is a retention
requirement, distinct from placement persistence (a storage capability). The
GitHub backend maps it to `retention-days` (whole days, 1..90); sub-day
durations round up to the 1-day minimum. Lifetime is intended to influence
placement selection where placement providers advertise supported retention.

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

Capabilities are the common currency of both selection phases (see Selection Model): backend
features, actor abilities, and inventory implementations (`impl.<id>`) all surface as
capabilities, and a rule is eligible only when every capability it requires is available.

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

The reference frontend is the Python package `ariadne`
(`frontends/python/ariadne/`); it authors Thread IR from semantic actions and an inventory,
then drives the engine in-process through the `ariadne.ariadne_core` PyO3 extension
(`crates/ariadne-py`). `loom` is the CLI binary (`crates/loom`), not the Python package. Both
are thin: the engine runs in-process and operates on Thread IR.

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

The engine is ONE library crate (`ariadne`, a module per phase); the CLI and the Python
extension are thin binaries over it. The phases are not shipped as separate crates.

```text
ariadne/                 root lib crate
  src/
    ir.rs                semantic IR types + WorkflowBuilder (source of truth)
    proto.rs             TIR serialization (prost wire types + serde JSON codec)
    diagnostics.rs
    select.rs            shared selection substrate:
                           Capability, Stability, Candidate, Registry<C>, resolve
    validate.rs
    lowering/            implementation selection (plan-time, backend-agnostic)
      mod.rs               Registry = select::Registry<LoweringDef>, LoweringBody, select
      scm.rs build.rs test.rs scan.rs sign.rs package.rs forge.rs   built-in packs
    planner.rs           Plan + LogicalOp (SemanticOp + ci.* ops); runs lowering selection
    cost.rs profile.rs analysis.rs optimize/   optional optimization
    backends/
      mod.rs             Backend trait; Catalogue = select::Registry<Instruction>; Selector
      renderers.rs       backend-agnostic YAML/Bash renderers
      github/            GitHub Actions backend (+ instructions.rs catalogue)
      local/             local Podman backend + executor (loom test)
crates/
  loom/                  CLI binary over Thread IR
  ariadne-py/            PyO3 extension; builds as module ariadne.ariadne_core
frontends/
  python/ariadne/        Python authoring frontend (semantic actions + Inventory)
```

Logical layering (modules flow inward to outward), with `select` as the shared base both
selection sites stand on:

```text
ir            select
    ↑           ↑
diagnostics     │
    ↑           │
validate        │
    ↑           │
lowering  ──────┘   (implementation selection)
    ↑
planner
    ↑
backends  (instruction selection, also on select)
    ↑
loom / ariadne-py
```

`ir` must not depend on planner, backends, or filesystem/Docker code.

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

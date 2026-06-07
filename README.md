# Ariadne

Ariadne is a CI/CD workflow **planning compiler**. You describe *what* your
pipeline is made of - artifacts, actions, effects, constraints, policies - and
Ariadne figures out *how* to run it: it validates the workflow, builds a correct
execution plan, optimizes artifact movement and runner placement, and emits
backend-specific CI configuration (e.g. GitHub Actions YAML).

You author **semantic intent** (`build.binary`, `scm.checkout`, `test.unit`),
never tool commands. The inventory declares which implementations are available
(cargo, maturin, pytest, git, ...); Ariadne selects one and lowers it.

> Ariadne owns planning. Frontends own ergonomics. Backends own emission.

**Thread IR (TIR)** is the stable interchange format between frontends and the
engine. **Loom** is the reference command-line frontend over TIR; a **Python
frontend** (the `ariadne` package) is provided for authoring workflows in code.

---

## Why

Existing CI systems make you hand-author execution mechanics: jobs, `needs`,
matrices, artifact upload/download steps, caches, runner labels. This is
tedious to get right and easy to get subtly wrong.

Ariadne lets you state workflow *semantics* and derives the execution plan:

```
  Author intent  →  Thread IR  →  Ariadne  →  Execution plan  →  Backend config
                                  (validate, analyze,
                                   plan, optimize)
```

The guarantee at the center of the design: **Ariadne always produces a correct
plan if one exists.** Optimization is optional and never required for
correctness. The baseline plan uses plain copy/upload/download semantics, and
richer strategies (mounts, colocation, fusion) are layered on top only when they
are provably safe.

## Core concepts

Thread IR has five first-class semantic entities:

| Entity | What it is |
|--------|------------|
| **Artifact** | Typed, immutable logical data (Wheel, Binary, ContainerImage, SBOM, TestReport, …) |
| **Action** | Computation that consumes artifacts, produces artifacts, may emit effects |
| **Effect** | External mutation / privileged interaction (Deployment, PublishRelease, SecretAccess, GitWrite) |
| **Placement** | How an artifact is physically realized/accessed (artifact store, shared volume, cache, registry, local path) |
| **Actor** | An execution resource (github-ubuntu, self-hosted-gpu, local-docker) |

These are deliberately *not* collapsed into generic jobs/steps. An action names
a **semantic operation** (e.g. `build.binary`), not a command. A raw `shell(...)`
escape hatch exists for tool-specific work, with typed boundaries the compiler
trusts (declared inputs, outputs, effects, secrets).

## From intent to a step: two-layer selection

Every operation in a plan has one uniform identity - a `dialect.action` (the
*logical op*) - specified to an implementation as `dialect.action.impl` (the
*specified op*), then lowered to a backend step:

```
Logical op        dialect.action          e.g. build.binary, scm.checkout, ci.artifact.upload
    ↓   implementation selection  (plan-time, backend-agnostic)
Specified op      dialect.action.impl     e.g. build.binary.cargo, scm.checkout.git
    ↓   instruction selection     (emit-time, backend-aware)
Native step       run: / uses:
```

- **Implementation selection** (`src/lowering/`) picks the tool from the
  inventory: the same `test.unit` becomes `test.unit.cargo` under `use("cargo")`
  or `test.unit.pytest` under `use("pytest")`.
- **Instruction selection** (each backend's catalogue) keys on
  `(action, implementation)` and renders the native step. `scm.checkout` becomes
  `uses: actions/checkout@v4` on GitHub or `git checkout .` elsewhere;
  `build.binary.cargo` runs `cargo build ...`.

Native-with-shell-fallback is the universal model: every semantic op carries a
shell `fallback` any backend can run, and a backend may *upgrade* it to a native
action when the inventory permits. The user-domain dialects (`scm`, `build`,
`test`, `fmt`, `docs`, `coverage`, `scan`, `sign`, `package`, `forge`) name
intent; the **`ci` dialect** names orchestration (`ci.artifact.upload` /
`download` / `transfer`, `ci.cache.restore` / `save`, `ci.approval`).

## How it works

The engine is a phase pipeline. Each phase is a module; data flows inward to
outward:

```
ir → diagnostics → validate → analysis → lowering → planner → optimize → backends
```

1. **validate** - semantic checks, type/effect safety, structured diagnostics.
2. **analysis** - purity, consumer counts, effect barriers (the safety substrate
   for reordering).
3. **lowering** - implementation selection: choose an implementation per semantic
   action from the inventory (backend-agnostic).
4. **planner** - produces a *correct baseline* plan (pure copy/upload-download).
5. **optimize** - a stack of passes, gated by compiler-style `-O` levels, each
   semantics-preserving with a legal fallback:
   - `placement` - copy → mount when the backend and actors support it
   - `actor` - utilization-aware runner right-sizing (profile-guided)
   - `colocation` - same-host path when producer and consumer share a runner
   - `parallelization` - enforce `max_parallel_jobs`
   - `dedup` / `fusion` / `sibling_fusion` - eliminate redundant work and pack
     independent same-runner jobs (effect-aware, cost-arbitrated)
6. **backends** - instruction selection + emission (GitHub Actions YAML, or a
   local Podman backend used for testing CI itself).

Profiles (observed durations, sizes, costs, utilization) influence cost
estimates only, **never** semantics.

See **[DESIGN.md](DESIGN.md)** for the full specification.

## Building from source

Requires a recent stable Rust toolchain. Local execution (`loom test` against
Podman) additionally requires [Podman](https://podman.io/); all other commands
are pure and need no container runtime.

```sh
git clone <repo-url> ariadne
cd ariadne
cargo build --release          # binary at target/release/loom
cargo test --workspace         # workspace = engine + loom + Python extension
```

The Python frontend builds with [maturin](https://www.maturin.rs/):

```sh
cd frontends/python
maturin develop                # installs the `ariadne` package + native extension
pytest tests/
```

## Quickstart

Workflows are consumed as Thread IR: either `.json` (serde) or `.pb` (protobuf
binary). An example lives at `tests/fixtures/simple-build-test.tir.json`.

```sh
# Is this workflow valid?
loom check tests/fixtures/simple-build-test.tir.json

# What plan should be produced? Emit GitHub Actions YAML.
loom plan tests/fixtures/simple-build-test.tir.json github

# Why this plan? Show planner + optimization decisions, the specified op for
# each step (build.binary.cargo: cargo build ...), and instruction selection
# when you name a backend.
loom explain tests/fixtures/simple-build-test.tir.json github

# Does it behave correctly? Run the workflow's test suite, or execute once
# in Podman if no suite is configured.
loom test tests/fixtures/simple-build-test.tir.json
```

### Optimization levels

`plan` and `explain` take a compiler-style `-O` flag (default `-O2`):

| Level | Passes |
|-------|--------|
| `-O0` | none - the correctness baseline (pure copy) |
| `-O1` | placement + actor optimization |
| `-O2` | + colocation + parallelization *(default)* |
| `-O3` | + deduplication + fusion (vertical + sibling) |

Feed a profile to guide cost-based decisions:

```sh
loom plan tests/fixtures/simple-build-test.tir.json local -O3 --profile profile.json
```

### Profile-guided planning

`loom profile` reads a backend's real run telemetry (durations, artifact sizes,
runner cost) and aggregates it into a profile the planner consumes. The loop is
self-sustaining: a workflow can collect its own timings, commit them, and plan
the next generation against measured cost.

```sh
loom profile github --workflow ci.yml --runs 20 --out profile.json
```

### Checking in generated CI

`loom plan --out` writes emission to a file, so you can keep generated CI under
version control and fail CI when it drifts:

```sh
loom plan tests/fixtures/simple-build-test.tir.json github --out .github/workflows/ci.yml
git diff --exit-code .github/workflows/ci.yml
```

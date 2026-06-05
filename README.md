# Ariadne

Ariadne is a CI/CD workflow **planning compiler**. You describe *what* your
pipeline is made of - artifacts, actions, effects, constraints, policies - and
Ariadne figures out *how* to run it: it validates the workflow, builds a correct
execution plan, optimizes artifact movement and runner placement, and emits
backend-specific CI configuration (e.g. GitHub Actions YAML).

**Loom** is the reference command-line frontend. It's a small, language-agnostic
CLI over **Thread IR (TIR)** - the stable interchange format between frontends
and the engine.

> Ariadne owns planning. Frontends own ergonomics. Backends own emission.

---

## Why

Existing CI systems make you hand-author execution mechanics: jobs, `needs`,
matrices, artifact upload/download steps, caches, runner labels. That's a plan
written by hand - tedious to get right and easy to get subtly wrong.

Ariadne lets you state workflow *semantics* and derives the execution plan:

```
  Author intent  →  Thread IR  →  Ariadne  →  Execution plan  →  Backend config
                                  (validate, analyze,
                                   plan, optimize)
```

The guarantee at the center of the design: **Ariadne always produces a correct
plan if one exists.** Optimization is optional and never required for
correctness - the baseline plan uses plain copy/upload/download semantics, and
richer strategies (mounts, colocation, fusion) are layered on top only when they
are provably safe.

## Core concepts

Thread IR has five first-class semantic entities - deliberately *not* collapsed
into generic "jobs" and "steps":

| Entity | What it is |
|--------|------------|
| **Artifact** | Typed, immutable logical data (Wheel, Binary, ContainerImage, SBOM, TestReport, …) |
| **Action** | Computation that consumes artifacts, produces artifacts, may emit effects |
| **Effect** | External mutation / privileged interaction (Deployment, PublishRelease, SecretAccess, GitWrite) |
| **Placement** | How an artifact is physically realized/accessed (artifact store, shared volume, cache, registry, local path) |
| **Actor** | An execution resource (github-ubuntu, self-hosted-gpu, local-docker) |

## How it works

The engine is a phase pipeline. Each phase is a module; data flows inward to
outward:

```
ir → diagnostics → validate → analysis → planner → optimize → backends
```

1. **validate** - semantic checks, type/effect safety, structured diagnostics.
2. **analysis** - purity, consumer counts, effect barriers (the safety substrate
   for reordering).
3. **planner** - produces a *correct baseline* plan (pure copy/upload-download).
4. **optimize** - a stack of passes, gated by compiler-style `-O` levels, each
   semantics-preserving with a legal fallback:
   - `placement` - copy → mount when the backend and actors support it
   - `actor` - utilization-aware runner right-sizing (profile-guided)
   - `colocation` - same-host path when producer and consumer share a runner
   - `parallelization` - enforce `max_parallel_jobs`
   - `dedup` / `fusion` - eliminate redundant and chained-through work (effect-aware)
5. **backends** - instruction selection + emission (GitHub Actions YAML, or a
   local Podman backend used for testing CI itself).

Profiles (observed durations, sizes, costs, utilization) influence cost
estimates only - **never** semantics.

See **[DESIGN.md](DESIGN.md)** for the full specification.

## Building from source

Requires a recent stable Rust toolchain. Local execution (`loom test` against
Podman) additionally requires [Podman](https://podman.io/); all other commands
are pure and need no container runtime.

```sh
git clone <repo-url> ariadne
cd ariadne
cargo build --release          # binary at target/release/loom
cargo test --workspace         # or: cargo t  (alias)
```

## Quickstart

Workflows are consumed as Thread IR - either `.json` (serde) or `.pb` (protobuf
binary). An example lives at `tests/fixtures/simple-build-test.tir.json`.

These examples use `loom` (the release binary lives at `target/release/loom`)
and the bundled example workflow.

```sh
# Is this workflow valid?
loom check tests/fixtures/simple-build-test.tir.json

# What plan should be produced? Emit GitHub Actions YAML.
loom plan tests/fixtures/simple-build-test.tir.json github

# Why this plan? Show planner + optimization decisions (and instruction
# selection if you name a backend).
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
| `-O3` | + deduplication + fusion |

Feed a profile to guide cost-based decisions:

```sh
loom plan tests/fixtures/simple-build-test.tir.json local -O3 --profile profile.json
```

### Checking in generated CI

`loom plan --out` writes emission to a file, so you can keep generated CI under
version control and fail CI when it drifts:

```sh
loom plan tests/fixtures/simple-build-test.tir.json github --out .github/workflows/ci.yml
git diff --exit-code .github/workflows/ci.yml
```

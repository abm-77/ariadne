# CLAUDE.md

This file gives Claude Code project-specific guidance for working on Ariadne and Loom.

## General Rules
- Never commit or push without explicit permission.
- Prioritize separation of concerns. I.e., TIR is backend-agnostic, it should not depend on any backend implementation details, that's what context-aware lowering is for. 
  Apply this broadly.
- Test everything we add.

## Project Summary

Ariadne is a CI/CD workflow planning engine written in Rust.

It consumes Thread IR, validates workflow semantics, generates a correct execution plan, optionally optimizes it, and emits backend-specific CI configurations such as GitHub Actions YAML.

Loom is the reference frontend and CLI for authoring workflows and interacting with Ariadne.

The project thesis:

> CI/CD is a planning problem, not a YAML authoring problem.

## Core Concepts

Thread IR has five first-class semantic entities:

- Artifact: typed immutable logical data, such as Wheel, Binary, ContainerImage, SBOM, TestReport, Model.
- Action: computation that consumes artifacts, produces artifacts, and may carry consequences.
- Consequence: external mutation or privileged interaction, such as Network, SecretAccess, GitWrite, PublishRelease, Deployment, CommentOnPr.
- Placement: physical realization/access strategy for an artifact, such as GitHub artifact, shared volume, cache, registry, local path.
- Actor: execution resource, such as github-ubuntu, self-hosted-gpu, local-docker.

Do not collapse these concepts into generic jobs/steps. The distinction is central to the design.

## Architectural Rule

Ariadne owns planning.

Frontends own ergonomics.

Backends own emission.

Thread IR is the boundary between frontends and Ariadne.

## Logical Op Model

Every operation in a plan has one uniform identity: a `dialect.action` (the
logical op), specified to an implementation as `dialect.action.impl` (the
specified op), then lowered to a backend physical step. Two phases on one shared
engine (`select.rs`): specification (`specifications/`, plan-time, backend-agnostic)
binds the logical op to an implementation, then instruction selection (each
backend's `Catalogue`, emit-time) lowers it to a native step. Instruction
selection keys on `(action, implementation)`.

- `LogicalOp::SemanticOp { action, implementation, label, args, fallback, env }`
  is the user-domain compute carrier (build/test/fmt/scm.checkout/package.publish/...).
- Orchestration primitives are the `ci.*` dialect, kept as typed `LogicalOp`
  variants (`UploadArtifact`, `DownloadArtifact`, `TransferArtifact`, cache,
  approval) so optimizer rewrites stay exhaustive. `RunShell` is the `shell.run`
  escape hatch.
- Every op answers `action()` / `implementation()`; do not reintroduce per-op
  matcher special-casing. A SemanticOp always carries a shell `fallback`; a
  backend may upgrade it to a native step (`uses:`) when an `impl.<id>` capability
  is present, else the `AnySemantic` fallback runs the shell. This is what keeps
  a legal plan always available (the correctness invariant).
- Never bake a tool command into the IR or emit a backend-specific step at plan
  time. Add new tools as lowerings, new native steps as backend instructions.

## Correctness Invariant

Ariadne must always produce a correct plan if one exists.

Optimization is optional. Correctness is mandatory.

A legal fallback plan should use copy/upload/download semantics when richer placement strategies such as mounts or colocation are unavailable.

Never make an optimization required for correctness.

## Implementation Language

Use Rust.

Do not introduce LLVM or MLIR. The hard problem is domain planning, not machine-code lowering.

## Workspace Layout

Two crates only. The engine is ONE library crate; the CLI is a thin binary.
We do not ship the phases independently. `ariadne` is the lib; frontend bindings
link it to run the engine in-process.

```text
ariadne/            (root lib crate; module per phase)
  src/
    ir.rs           semantic IR types + WorkflowBuilder (source of truth)
    proto.rs        TIR serialization: prost wire types + serde JSON codec
    diagnostics.rs
    select.rs       shared selection substrate (Capability, Candidate, Registry, resolve)
    validate.rs
    specifications/ specification (plan-time): semantic action -> impl
    planner.rs      Plan + LogicalOp (SemanticOp + ci.* ops); runs specification
    cost.rs profile.rs analysis.rs optimize/   optional optimization
    backends/
      mod.rs        Backend trait, Catalogue, Selector, instruction selection
      renderers.rs  backend-agnostic YAML/Bash renderers
      github/       GitHub Actions backend (+ instructions.rs catalogue)
      local/        local Podman backend + executor + assertions (loom test)
crates/
  loom/             CLI binary over TIR (depends on ariadne)
  ariadne-py/       PyO3 extension; builds as module ariadne.ariadne_core
frontends/
  python/ariadne/   Python authoring frontend (semantic actions + Inventory)
```

Keep the logical layering even though it is one crate. Modules flow inward to
outward: `ir -> diagnostics -> validate -> specification -> planner -> backends`,
with `proto` beside `ir` and `select` as the shared base both selection sites
(specification, backends) stand on. Do not let `ir` depend on planner, backends, or
filesystem/Docker code.

## TIR Serialization

The Rust `ir` types are the SINGLE source of truth. There is no hand-authored
`.proto`. TIR has two wire formats: protobuf binary (via prost wire types in
`proto`, defined in Rust) and JSON (serde over the `ir` types). A `.proto` for
non-Rust frontends is generated from the Rust types when needed, never the
reverse. Files: `.pb` (binary), `.json` (JSON).

## Testing Strategy

Favor small, phase-specific tests and golden fixtures.

Test categories:

- Thread IR serialization/deserialization
- validation
- type checking
- consequence safety
- placement fallback
- placement optimization
- policy constraints
- backend emission
- local execution
- profile-guided planning

Important invariant tests:

- backend lacks mounts -> copy fallback works with warning
- backend supports mounts -> large shared artifact uses mount when legal
- deployment consequence -> not deduplicated or reordered before tests
- policy max_parallel_jobs = 10 -> plan never exceeds 10
- GitHub hosted backend -> cross-job mount unavailable -> upload/download fallback

## Local CI Testing Feature

A key product feature is testing CI itself.

Loom should eventually support:

```bash
loom check
loom plan
loom test
loom emit --check
loom simulate
```

`loom test` should support assertions over:

- expected artifacts
- expected consequences
- policy compliance
- backend compatibility
- event fixtures
- generated backend output
- unsafe consequence gating

Event fixtures should allow testing contexts such as:

- pull_request
- fork pull request
- push-main
- tag-release

## Design Preferences

Prefer explicit semantic types over strings where reasonable.

Prefer stable IDs over references in graph structures.

Prefer structured diagnostics over plain strings.

Prefer creating a correct naive plan before adding optimization passes.

Prefer making plans explainable over making them maximally clever.

Prefer testable deterministic planner behavior.

## Things To Avoid

Avoid making Ariadne a CI provider.

Avoid adding cluster scheduling/provisioning responsibilities.

Avoid frontend syntax bikeshedding in core crates.

Avoid making Loom required to use Ariadne.

Avoid backend-specific assumptions in Thread IR.

Avoid optimizing before there is a correctness baseline.

Avoid hiding privileged behavior inside shell actions without declaring consequences.

## Shell Actions

Shell execution should be treated as an escape hatch with typed boundaries.

Shell actions should declare:

- inputs
- outputs
- consequences
- environment
- secrets
- capture rules
- failure policy

The compiler should only trust declared boundaries.

## Naming

- Ariadne: core planning engine
- Loom: reference frontend/CLI
- Thread IR: canonical semantic graph/interchange representation

Use these names consistently.


## Code Style 
- Do not use section header comments like // ---- TITLE ----, or any variation of including:
// ---------------------------------------------------------------------------
// Object-safe trait for registry and CLI use
// ---------------------------------------------------------------------------
- Keep comments to at most 4 lines, if you can't explain it in that, you're being too verbose.
- Do not use file-level comments explaining what a file does.
- Do not, under any circumstance, use an em-dash.

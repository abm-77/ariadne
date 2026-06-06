//! The optimization phase: passes that transform a *correct* baseline plan into
//! a better-but-equivalent execution plan. Every pass must preserve semantics
//! and leave a legal fallback; optimization is never required for correctness.

use crate::analysis::Analysis;
use crate::backends::BackendCapabilities;
use crate::ir::{Objective, Policies, Workflow};
use crate::planner::{OptimizationDecision, Plan};
use crate::profile::Profile;

/// Compiler-style optimization levels. `O0` is the untouched baseline; higher
/// levels enable progressively more (and more aggressive) passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum OptLevel {
    /// No optimization — the correctness baseline (pure copy/upload-download).
    O0,
    /// Placement + actor optimization (safe, high value).
    O1,
    /// + colocation + parallelization. The default.
    #[default]
    O2,
    /// + hoisting + deduplication + fusion (effect-aware reordering).
    O3,
}

impl OptLevel {
    pub fn from_u8(n: u8) -> Self {
        match n {
            0 => OptLevel::O0,
            1 => OptLevel::O1,
            2 => OptLevel::O2,
            _ => OptLevel::O3,
        }
    }
}

/// Everything a pass needs: the source workflow (placements, actors), profile
/// data, backend capabilities, policy limits, static analysis, the objective
/// ordering, and the active level.
pub struct OptimizeCtx<'a> {
    pub workflow: &'a Workflow,
    pub profile: &'a Profile,
    pub backend_caps: BackendCapabilities,
    pub policy: &'a Policies,
    pub analysis: &'a Analysis,
    pub objectives: Vec<Objective>,
    pub level: OptLevel,
}

/// One optimization pass. `run` mutates the plan in place and returns the
/// decisions it made (recorded for `loom explain`).
pub trait Pass {
    fn name(&self) -> &str;
    /// Lowest `-O` level at which this pass is enabled.
    fn min_level(&self) -> OptLevel;
    fn run(&self, plan: &mut Plan, ctx: &OptimizeCtx) -> Vec<OptimizationDecision>;
}

mod actor;
mod colocation;
mod dedup;
mod fusion;
mod parallelization;
mod placement;
mod sibling_fusion;

/// Registered passes in execution (safety) order. Reordering passes (dedup,
/// fusion) come last and are gated by analysis barriers.
fn passes() -> Vec<Box<dyn Pass>> {
    vec![
        Box::new(placement::PlacementPass),
        Box::new(actor::ActorPass),
        Box::new(colocation::ColocationPass),
        Box::new(parallelization::ParallelizationPass),
        Box::new(dedup::DeduplicationPass),
        Box::new(fusion::FusionPass),
        Box::new(sibling_fusion::SiblingFusionPass),
    ]
}

/// Run the optimization pipeline at the ctx's level. At `O0`, or when no pass
/// applies, the plan is returned unchanged — the correctness baseline.
pub fn optimize(mut plan: Plan, ctx: &OptimizeCtx) -> Plan {
    if ctx.level == OptLevel::O0 {
        return plan;
    }
    for pass in passes() {
        if ctx.level >= pass.min_level() {
            let decisions = pass.run(&mut plan, ctx);
            plan.optimizations.extend(decisions);
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{ArtifactType, WorkflowBuilder};

    fn ctx<'a>(
        wf: &'a crate::ir::Workflow,
        profile: &'a Profile,
        analysis: &'a Analysis,
        policy: &'a Policies,
        level: OptLevel,
    ) -> OptimizeCtx<'a> {
        OptimizeCtx {
            workflow: wf,
            profile,
            backend_caps: BackendCapabilities::default(),
            policy,
            analysis,
            objectives: crate::ir::default_objectives(),
            level,
        }
    }

    fn sample() -> (Plan, crate::ir::Workflow) {
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::SourceTree);
        let bin = b.artifact("bin", ArtifactType::Binary);
        b.shell_action("checkout", "checkout", &[], &[src], "x");
        b.shell_action("build", "build", &[src], &[bin], "make");
        b.actor("l", &["ubuntu-latest"], &[]);
        let wf = b.build();
        (crate::planner::plan(&wf).unwrap(), wf)
    }

    #[test]
    fn empty_pipeline_is_identity() {
        let (plan, wf) = sample();
        let (profile, policy) = (Profile::default(), wf.policies.clone());
        let analysis = Analysis::of(&wf);
        let baseline_units = plan.units.len();
        let out = optimize(plan, &ctx(&wf, &profile, &analysis, &policy, OptLevel::O2));
        assert_eq!(out.units.len(), baseline_units);
        assert!(out.optimizations.is_empty());
    }

    #[test]
    fn o0_short_circuits() {
        let (plan, wf) = sample();
        let (profile, policy) = (Profile::default(), wf.policies.clone());
        let analysis = Analysis::of(&wf);
        let out = optimize(plan, &ctx(&wf, &profile, &analysis, &policy, OptLevel::O0));
        assert!(out.optimizations.is_empty());
    }

    #[test]
    fn level_ordering() {
        assert!(OptLevel::O2 >= OptLevel::O1);
        assert!(OptLevel::O0 < OptLevel::O3);
        assert_eq!(OptLevel::from_u8(0), OptLevel::O0);
        assert_eq!(OptLevel::from_u8(9), OptLevel::O3);
        assert_eq!(OptLevel::default(), OptLevel::O2);
    }
}

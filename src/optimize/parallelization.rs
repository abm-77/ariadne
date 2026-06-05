//! Parallelization under policy: enforce `max_parallel_jobs`. The baseline DAG
//! already lets independent units run concurrently; if its natural width would
//! exceed the policy cap N, we add ordering edges so no more than N units are
//! ever runnable at once.
//!
//! Enforcement adds edge `unit[i] -> unit[i-N]` over the topological order. That
//! partitions every unit into N totally-ordered chains (by index mod N), so by
//! Dilworth's theorem the widest antichain — i.e. peak concurrency — is at most
//! N. Edges only ever point backward in topo order, so no effect is moved
//! earlier than the baseline placed it.

use super::{OptLevel, OptimizeCtx, Pass};
use crate::planner::{OptimizationDecision, Plan};

pub struct ParallelizationPass;

impl Pass for ParallelizationPass {
    fn name(&self) -> &str { "parallelization" }
    fn min_level(&self) -> OptLevel { OptLevel::O2 }

    fn run(&self, plan: &mut Plan, ctx: &OptimizeCtx) -> Vec<OptimizationDecision> {
        let Some(n) = ctx.policy.max_parallel_jobs else { return Vec::new() };
        let natural = plan.max_concurrency();
        if n == 0 || natural <= n {
            return Vec::new(); // already within policy
        }

        let order: Vec<_> = plan.units.iter().map(|u| u.id).collect();
        for i in n..plan.units.len() {
            let dep = order[i - n];
            let needs = &mut plan.units[i].needs;
            if !needs.contains(&dep) {
                needs.push(dep);
            }
        }

        vec![OptimizationDecision {
            pass: "parallelization".into(),
            target: "schedule".into(),
            from: format!("up to {natural} concurrent"),
            to: format!("at most {n} concurrent"),
            reason: format!("serialized into {n}-wide chains to honor max_parallel_jobs={n}"),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analysis;
    use crate::backends::BackendCapabilities;
    use crate::ir::{default_objectives, ArtifactType, Workflow, WorkflowBuilder};
    use crate::optimize::{optimize, OptLevel, OptimizeCtx};
    use crate::profile::Profile;

    fn run(wf: &Workflow, level: OptLevel) -> Plan {
        let plan = crate::planner::plan(wf).unwrap();
        let profile = Profile::default();
        let analysis = Analysis::of(wf);
        let ctx = OptimizeCtx {
            workflow: wf,
            profile: &profile,
            backend_caps: BackendCapabilities::empty(),
            policy: &wf.policies,
            analysis: &analysis,
            objectives: default_objectives(),
            level,
        };
        optimize(plan, &ctx)
    }

    /// `n` independent actions (no data dependencies) → natural width n.
    fn fan_out(n: usize, cap: Option<usize>) -> Workflow {
        let mut b = WorkflowBuilder::new("w");
        for i in 0..n {
            let out = b.artifact(&format!("a{i}"), ArtifactType::Binary);
            b.shell_action(&format!("job{i}"), "build", &[], &[out], "make");
        }
        b.actor("r", &["ubuntu-latest"], &[]);
        if let Some(c) = cap {
            b.max_parallel_jobs(c);
        }
        b.build()
    }

    #[test]
    fn caps_concurrency_to_policy() {
        let plan = run(&fan_out(5, Some(2)), OptLevel::O2);
        assert!(plan.max_concurrency() <= 2, "width {}", plan.max_concurrency());
        assert!(plan.optimizations.iter().any(|d| d.pass == "parallelization"));
    }

    #[test]
    fn within_policy_adds_no_edges() {
        let plan = run(&fan_out(3, Some(5)), OptLevel::O2);
        assert!(plan.units.iter().all(|u| u.needs.is_empty()));
        assert!(plan.optimizations.iter().all(|d| d.pass != "parallelization"));
    }

    #[test]
    fn no_policy_is_noop() {
        let plan = run(&fan_out(5, None), OptLevel::O2);
        assert!(plan.units.iter().all(|u| u.needs.is_empty()));
    }

    #[test]
    fn only_runs_at_o2() {
        let plan = run(&fan_out(5, Some(2)), OptLevel::O1);
        assert!(plan.max_concurrency() > 2, "O1 should not enforce");
    }

    #[test]
    fn enforcement_chains_units_n_apart() {
        let plan = run(&fan_out(4, Some(2)), OptLevel::O2);
        // unit[2] depends on unit[0], unit[3] on unit[1].
        assert!(plan.units[2].needs.contains(&plan.units[0].id));
        assert!(plan.units[3].needs.contains(&plan.units[1].id));
    }
}

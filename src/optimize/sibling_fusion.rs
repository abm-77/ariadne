//! Sibling (horizontal) fusion: pack independent units that share a runner into
//! one job. Unlike producer->consumer fusion, this trades parallelism for fewer
//! jobs, so it is never an unconditional win. The cost model is the arbiter: a
//! merge is committed only if it strictly lowers the plan's objective-ordered
//! cost. Under a concurrency cap the siblings serialize regardless, so packing
//! removes their redundant setup/transfer for free; with runners to spare,
//! packing lengthens the makespan and is rejected.
//!
//! Only pure, non-barrier units are eligible, so the arbitrary serialization
//! order within a packed job can never reorder an effect.

use super::{OptLevel, OptimizeCtx, Pass};
use crate::cost::Cost;
use crate::planner::{OptimizationDecision, Plan};
use std::cmp::Ordering;
use std::collections::HashSet;
use ustr::Ustr;

pub struct SiblingFusionPass;

impl Pass for SiblingFusionPass {
    fn name(&self) -> &str {
        "sibling_fusion"
    }
    fn min_level(&self) -> OptLevel {
        OptLevel::O3
    }

    fn run(&self, plan: &mut Plan, ctx: &OptimizeCtx) -> Vec<OptimizationDecision> {
        let mut decisions = Vec::new();
        // Greedy hill-climb: each round commit the single best improving merge,
        // then re-evaluate (a merge can expose or foreclose others).
        loop {
            let base = Cost::estimate(plan, ctx.profile);
            let mut best: Option<(usize, usize, Cost)> = None;
            for i in 0..plan.units.len() {
                for j in (i + 1)..plan.units.len() {
                    if !fusible(plan, ctx, i, j) {
                        continue;
                    }
                    let trial = merged(plan, i, j);
                    let cost = Cost::estimate(&trial, ctx.profile);
                    if cost.cmp_by(&base, &ctx.objectives) != Ordering::Less {
                        continue; // not a strict improvement
                    }
                    if best
                        .as_ref()
                        .is_none_or(|(_, _, b)| cost.cmp_by(b, &ctx.objectives) == Ordering::Less)
                    {
                        best = Some((i, j, cost));
                    }
                }
            }
            let Some((i, j, _)) = best else { break };
            let from = format!(
                "{} | {}",
                plan.units[i].action_name, plan.units[j].action_name
            );
            let packed = plan.units[i].action_name.to_string();
            *plan = merged(plan, i, j);
            // The merged unit took unit i's id.
            let target = plan
                .units
                .iter()
                .find(|u| u.action_name.as_str().starts_with(&packed))
                .map(|u| u.id.to_string())
                .unwrap_or_default();
            decisions.push(OptimizationDecision {
                pass: "sibling_fusion".into(),
                target,
                from,
                to: "one job".into(),
                reason: "packed independent units sharing a runner; cost improved under the objective order".into(),
            });
        }
        decisions
    }
}

/// Two units may be packed if they share a runner, are both pure (so order
/// among them is irrelevant) and neither barrier, and are independent (no
/// dependency path either way, which also guarantees the merge stays acyclic).
fn fusible(plan: &Plan, ctx: &OptimizeCtx, i: usize, j: usize) -> bool {
    let (a, b) = (&plan.units[i], &plan.units[j]);
    a.runner == b.runner
        && shares_toolchain(a, b)
        && ctx.analysis.is_pure(a.action_id)
        && ctx.analysis.is_pure(b.action_id)
        && !ctx.analysis.is_barrier(a.action_id)
        && !ctx.analysis.is_barrier(b.action_id)
        && !depends_on(plan, a.id, b.id)
        && !depends_on(plan, b.id, a.id)
}

/// Heuristic: only pack units with the *same* build environment — identical
/// toolchain sets (both Rust, both Python, or both bare). Requiring equality
/// (not mere overlap) keeps a mixed-toolchain unit, e.g. one that builds a Rust
/// cdylib and packages a Python wheel, from snowballing every job onto one
/// runner; packing groups cleanly by language.
fn shares_toolchain(a: &crate::planner::ExecutionUnit, b: &crate::planner::ExecutionUnit) -> bool {
    let norm = |u: &crate::planner::ExecutionUnit| {
        let mut t: Vec<String> = u.toolchains.iter().map(|t| t.to_string()).collect();
        t.sort();
        t.dedup();
        t
    };
    norm(a) == norm(b)
}

/// Does `x` transitively need `y` (a path x -> ... -> y along `needs`)?
fn depends_on(plan: &Plan, x: Ustr, y: Ustr) -> bool {
    let mut stack = vec![x];
    let mut seen = HashSet::new();
    while let Some(cur) = stack.pop() {
        let Some(u) = plan.units.iter().find(|u| u.id == cur) else {
            continue;
        };
        for n in &u.needs {
            if *n == y {
                return true;
            }
            if seen.insert(*n) {
                stack.push(*n);
            }
        }
    }
    false
}

/// A clone of `plan` with unit `j` merged into unit `i` (which keeps its id).
/// Ops are concatenated and de-duplicated of redundant input pulls; needs and
/// secrets are unioned; downstream `needs` on `j` are rewired to `i`.
fn merged(plan: &Plan, i: usize, j: usize) -> Plan {
    let mut out = plan.clone();
    let donor = out.units.remove(j);
    // `i` shifts left if it was after `j`; but j > i always here, so `i` holds.
    let host = &mut out.units[i];
    let host_id = host.id;

    host.ops.extend(donor.ops);
    super::fusion::dedup_input_pulls(&mut host.ops);
    for n in donor.needs {
        if n != host_id && !host.needs.contains(&n) {
            host.needs.push(n);
        }
    }
    for s in donor.secrets {
        if !host.secrets.contains(&s) {
            host.secrets.push(s);
        }
    }
    for d in donor.dependencies {
        if !host.dependencies.contains(&d) {
            host.dependencies.push(d);
        }
    }
    for t in donor.toolchains {
        if !host.toolchains.contains(&t) {
            host.toolchains.push(t);
        }
    }
    host.action_name = Ustr::from(&format!("{}-and-{}", host.action_name, donor.action_name));

    let donor_id = donor.id;
    for u in &mut out.units {
        if u.needs.contains(&donor_id) {
            u.needs.retain(|n| *n != donor_id);
            if !u.needs.contains(&host_id) {
                u.needs.push(host_id);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analysis;
    use crate::backends::BackendCapabilities;
    use crate::ir::{ArtifactType, ConsequenceKind, Workflow, WorkflowBuilder, default_objectives};
    use crate::optimize::{OptLevel, OptimizeCtx, optimize};
    use crate::profile::Profile;

    use crate::ir::Objective;

    /// Optimize with an explicit objective order and profile, so we exercise
    /// sibling fusion directly without the parallelization pass adding edges.
    fn run_with(
        wf: &Workflow,
        objectives: Vec<Objective>,
        profile: &Profile,
        level: OptLevel,
    ) -> Plan {
        let plan = crate::planner::plan(wf).unwrap();
        let analysis = Analysis::of(wf);
        let ctx = OptimizeCtx {
            workflow: wf,
            profile,
            backend_caps: BackendCapabilities::empty(),
            policy: &wf.policies,
            analysis: &analysis,
            objectives,
            level,
        };
        optimize(plan, &ctx)
    }

    /// checkout fans out to `n` independent sibling jobs on one runner. Each
    /// sibling is a distinct computation (own action name + script) so dedup's
    /// CSE leaves them alone and only sibling fusion can pack them.
    fn fan_out(n: usize) -> Workflow {
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::Binary);
        b.shell_action("checkout", "checkout", &[], &[src], "co");
        for i in 0..n {
            let out = b.artifact(&format!("o{i}"), ArtifactType::Binary);
            let (id, script) = (format!("check{i}"), format!("lint{i}"));
            b.shell_action(&id, &id, &[src], &[out], &script);
        }
        b.actor("r", &["ubuntu-latest"], &[]);
        b.build()
    }

    /// A profile that prices runner time, so per-job setup overhead has a dollar
    /// cost the optimizer can save by packing.
    fn priced() -> Profile {
        let mut p = Profile::default();
        // Keyed by the actor's label (what `unit.runner` resolves to).
        p.runner_costs.insert("ubuntu-latest".into(), 0.01);
        p
    }

    #[test]
    fn packs_independent_siblings_when_cost_prioritized() {
        // Dollars-first, runners to spare: each packed sibling saves one job's
        // setup overhead, so the three siblings collapse onto one job.
        let objs = vec![Objective::DollarCost, Objective::CriticalPath];
        let plan = run_with(&fan_out(3), objs, &priced(), OptLevel::O3);
        // The three siblings pack; the second vertical pass then folds the sole
        // producer (checkout) into them, leaving a single job.
        assert_eq!(plan.units.len(), 1, "everything collapses onto one job");
        assert!(
            plan.optimizations
                .iter()
                .any(|d| d.pass == "sibling_fusion")
        );
    }

    #[test]
    fn keeps_siblings_parallel_under_latency_first() {
        // Latency-first with runners to spare: packing only lengthens the
        // makespan, so it must not fire.
        let plan = run_with(&fan_out(3), default_objectives(), &priced(), OptLevel::O3);
        assert_eq!(plan.units.len(), 4, "siblings should stay parallel");
        assert!(
            plan.optimizations
                .iter()
                .all(|d| d.pass != "sibling_fusion")
        );
    }

    #[test]
    fn does_not_run_below_o3() {
        let objs = vec![Objective::DollarCost, Objective::CriticalPath];
        let plan = run_with(&fan_out(3), objs, &priced(), OptLevel::O2);
        assert_eq!(plan.units.len(), 4);
    }

    #[test]
    fn never_packs_an_effectful_unit() {
        // Dollars-first would tempt packing, but a unit carrying an effect is not
        // pure and must stay its own job (its order must be preservable).
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::Binary);
        let o1 = b.artifact("o1", ArtifactType::Binary);
        let o2 = b.artifact("o2", ArtifactType::Binary);
        b.shell_action("checkout", "checkout", &[], &[src], "co");
        let ship = b.shell_action("ship", "ship", &[src], &[o1], "deploy");
        let eff = b.consequence("deploy", ConsequenceKind::Deployment, false);
        b.add_consequence_to(ship, eff);
        b.shell_action("lint", "lint", &[src], &[o2], "lint");
        b.actor("r", &["ubuntu-latest"], &[]);
        let objs = vec![Objective::DollarCost, Objective::CriticalPath];
        let plan = run_with(&b.build(), objs, &priced(), OptLevel::O3);
        assert!(
            plan.units.iter().any(|u| u.action_name.as_str() == "ship"),
            "effectful unit must remain unmerged"
        );
    }
}

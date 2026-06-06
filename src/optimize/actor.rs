//! Actor optimization: re-assign a unit's actor using profile utilization, to
//! right-size under-utilized actors and relieve over-utilized (contended) ones.
//! Profile data only — with no profile, this pass is a no-op, so it can never be
//! required for correctness. Pinned actors (`Specific`) and required
//! capabilities are always respected.

use super::{OptLevel, OptimizeCtx, Pass};
use crate::ir::{ActionCall, Actor, ActorConstraint, Workflow};
use crate::planner::{OptimizationDecision, Plan, actor_for};

/// At/above this utilization an actor is treated as contended.
const SATURATED: f64 = 0.85;
/// At/below this utilization an actor is over-provisioned for its work.
const UNDER: f64 = 0.30;

pub struct ActorPass;

impl Pass for ActorPass {
    fn name(&self) -> &str {
        "actor"
    }
    fn min_level(&self) -> OptLevel {
        OptLevel::O1
    }

    fn run(&self, plan: &mut Plan, ctx: &OptimizeCtx) -> Vec<OptimizationDecision> {
        let mut decisions = Vec::new();
        for unit in &mut plan.units {
            let action = ctx.workflow.action_call(unit.action_id);
            // Pinned actor, or no resolvable actor → leave as planned.
            let Some(candidates) = candidate_actors(action, ctx.workflow) else {
                continue;
            };
            let Some(current) = actor_for(action, ctx.workflow) else {
                continue;
            };
            // Profile-driven only: no utilization data ⇒ no change.
            let Some(cur_util) = ctx.profile.utilization(current.id.as_str()) else {
                continue;
            };

            let util = |a: &Actor| ctx.profile.utilization(a.id.as_str()).unwrap_or(0.0);
            let cost = |a: &Actor| {
                a.labels
                    .first()
                    .and_then(|l| ctx.profile.runner_costs.get(l.as_str()).copied())
                    .unwrap_or(f64::INFINITY)
            };
            let viable =
                |a: &Actor| a.id != current.id && caps_superset(a, current) && util(a) < SATURATED;

            let (choice, reason) = if cur_util >= SATURATED {
                let pick = candidates
                    .iter()
                    .copied()
                    .filter(|&a| viable(a))
                    .min_by(|&a, &b| util(a).total_cmp(&util(b)));
                (
                    pick,
                    format!(
                        "actor '{}' saturated (util={cur_util:.2}); moved to reduce contention",
                        current.id
                    ),
                )
            } else if cur_util <= UNDER {
                let cur_cost = cost(current);
                let pick = candidates
                    .iter()
                    .copied()
                    .filter(|&a| viable(a) && cost(a) < cur_cost)
                    .min_by(|&a, &b| cost(a).total_cmp(&cost(b)));
                (
                    pick,
                    format!(
                        "actor '{}' under-utilized (util={cur_util:.2}); moved to a cheaper fit",
                        current.id
                    ),
                )
            } else {
                (None, String::new())
            };

            if let Some(new) = choice {
                let from = current.id.to_string();
                if let Some(label) = new.labels.first() {
                    unit.runner = *label;
                }
                unit.actor_capabilities = new.capabilities.clone();
                decisions.push(OptimizationDecision {
                    pass: "actor".into(),
                    target: unit.id.to_string(),
                    from,
                    to: new.id.to_string(),
                    reason,
                });
            }
        }
        decisions
    }
}

/// Actors the action could legally run on, or `None` if it is pinned to a
/// specific actor (a hard constraint we never override). With only label
/// constraints, every actor carrying a required label qualifies; with none,
/// any actor does.
fn candidate_actors<'a>(action: &ActionCall, workflow: &'a Workflow) -> Option<Vec<&'a Actor>> {
    if action
        .actor_constraints
        .iter()
        .any(|c| matches!(c, ActorConstraint::Specific(_)))
    {
        return None;
    }
    let labels: Vec<_> = action
        .actor_constraints
        .iter()
        .filter_map(|c| match c {
            ActorConstraint::Label(l) => Some(*l),
            _ => None,
        })
        .collect();
    let actors: Vec<&Actor> = if labels.is_empty() {
        workflow.actors().iter().collect()
    } else {
        workflow
            .actors()
            .iter()
            .filter(|a| labels.iter().all(|l| a.labels.contains(l)))
            .collect()
    };
    Some(actors)
}

/// `a` provides every capability `current` does (never drop a capability the
/// unit may rely on, e.g. a mount upgraded by the placement pass).
fn caps_superset(a: &Actor, current: &Actor) -> bool {
    current
        .capabilities
        .iter()
        .all(|c| a.capabilities.contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analysis;
    use crate::backends::BackendCapabilities;
    use crate::ir::{ArtifactType, WorkflowBuilder, default_objectives};
    use crate::optimize::{OptLevel, OptimizeCtx, optimize};
    use crate::profile::Profile;

    fn run(wf: &Workflow, profile: Profile) -> Plan {
        let plan = crate::planner::plan(wf).unwrap();
        let analysis = Analysis::of(wf);
        let ctx = OptimizeCtx {
            workflow: wf,
            profile: &profile,
            backend_caps: BackendCapabilities::empty(),
            policy: &wf.policies,
            analysis: &analysis,
            objectives: default_objectives(),
            level: OptLevel::O1,
        };
        optimize(plan, &ctx)
    }

    /// Two actors sharing the "ci" constraint label; each leads with a distinct
    /// runner label (so runner_costs can differ). "big" resolves first.
    fn two_runner_wf() -> Workflow {
        let mut b = WorkflowBuilder::new("w");
        let out = b.artifact("out", ArtifactType::Binary);
        let act = b.shell_action("build", "build", &[], &[out], "make");
        b.actor("big", &["big-label", "ci"], &[]);
        b.actor("small", &["small-label", "ci"], &[]);
        b.constrain_label(act, "ci");
        b.build()
    }

    #[test]
    fn no_profile_is_noop() {
        let plan = run(&two_runner_wf(), Profile::default());
        assert!(plan.optimizations.iter().all(|d| d.pass != "actor"));
    }

    #[test]
    fn under_utilized_actor_moves_to_cheaper_fit() {
        let mut p = Profile::default();
        p.actor_utilization.insert("big".into(), 0.1); // current, idle and pricey
        p.runner_costs.insert("big-label".into(), 1.0);
        p.runner_costs.insert("small-label".into(), 0.2);
        let plan = run(&two_runner_wf(), p);
        let moved = plan.optimizations.iter().find(|d| d.pass == "actor");
        assert!(
            moved.is_some(),
            "expected an actor move; got {:?}",
            plan.optimizations
        );
        assert_eq!(moved.unwrap().to, "small");
    }

    #[test]
    fn saturated_actor_moves_to_relieve_contention() {
        let mut p = Profile::default();
        p.actor_utilization.insert("big".into(), 0.95); // current, saturated
        p.actor_utilization.insert("small".into(), 0.2); // idle alternative
        let plan = run(&two_runner_wf(), p);
        let moved = plan.optimizations.iter().find(|d| d.pass == "actor");
        assert!(
            moved.is_some(),
            "expected a contention move; got {:?}",
            plan.optimizations
        );
        assert_eq!(moved.unwrap().to, "small");
        assert_eq!(plan.units[0].runner.as_str(), "small-label");
    }

    #[test]
    fn pinned_actor_is_never_moved() {
        let mut b = WorkflowBuilder::new("w");
        let out = b.artifact("out", ArtifactType::Binary);
        let act = b.shell_action("build", "build", &[], &[out], "make");
        let big = b.actor("big", &["ci"], &[]);
        b.actor("small", &["ci"], &[]);
        b.constrain_actor(act, big); // Specific pin
        let mut p = Profile::default();
        p.actor_utilization.insert("big".into(), 0.99);
        p.actor_utilization.insert("small".into(), 0.0);
        let plan = run(&b.build(), p);
        assert!(plan.optimizations.iter().all(|d| d.pass != "actor"));
    }
}

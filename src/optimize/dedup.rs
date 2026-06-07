//! Deduplication: when two units perform the identical pure computation (same
//! dependencies, same ops, same inputs — outputs may be named differently), keep
//! one and drop the rest, remapping downstream consumers onto the survivor's
//! outputs. Effectful or barrier units are never deduplicated.

use super::{OptLevel, OptimizeCtx, Pass};
use crate::planner::{ExecutionUnit, LogicalOp, OptimizationDecision, Plan};
use std::collections::HashMap;
use ustr::Ustr;

pub struct DeduplicationPass;

impl Pass for DeduplicationPass {
    fn name(&self) -> &str {
        "dedup"
    }
    fn min_level(&self) -> OptLevel {
        OptLevel::O3
    }

    fn run(&self, plan: &mut Plan, ctx: &OptimizeCtx) -> Vec<OptimizationDecision> {
        let mut seen: Vec<(Vec<String>, Ustr, Vec<Ustr>)> = Vec::new();
        let mut drop_to_kept: HashMap<Ustr, Ustr> = HashMap::new();
        let mut out_remap: HashMap<Ustr, Ustr> = HashMap::new();
        let mut decisions = Vec::new();

        for u in &plan.units {
            // Only pure, non-barrier units take part — never collapse effects.
            if !ctx.analysis.is_pure(u.action_id) || ctx.analysis.is_barrier(u.action_id) {
                continue;
            }
            let sig = signature(u);
            let outs = outputs(u);
            if let Some((_, kept_id, kept_outs)) = seen.iter().find(|(s, _, _)| *s == sig)
                && kept_outs.len() == outs.len()
            {
                for (d, k) in outs.iter().zip(kept_outs) {
                    out_remap.insert(*d, *k);
                }
                drop_to_kept.insert(u.id, *kept_id);
                decisions.push(OptimizationDecision {
                    pass: "dedup".into(),
                    target: u.id.to_string(),
                    from: u.action_name.to_string(),
                    to: kept_id.to_string(),
                    reason: format!("identical pure computation to '{kept_id}'; deduplicated"),
                });
                continue;
            }
            seen.push((sig, u.id, outs));
        }

        if drop_to_kept.is_empty() {
            return decisions;
        }

        plan.units.retain(|u| !drop_to_kept.contains_key(&u.id));
        for u in &mut plan.units {
            let old = std::mem::take(&mut u.needs);
            for n in old {
                let mapped = drop_to_kept.get(&n).copied().unwrap_or(n);
                if mapped != u.id && !u.needs.contains(&mapped) {
                    u.needs.push(mapped);
                }
            }
            for op in &mut u.ops {
                if let LogicalOp::DownloadArtifact { name, .. }
                | LogicalOp::TransferArtifact { name, .. } = op
                    && let Some(k) = out_remap.get(name)
                {
                    *name = *k;
                }
            }
        }
        decisions
    }
}

/// Computation fingerprint: dependency set + ops, ignoring the RunShell label
/// (a job name, not part of the computation) and Upload outputs (what may differ).
fn signature(u: &ExecutionUnit) -> Vec<String> {
    let mut sig: Vec<String> = u.needs.iter().map(|n| format!("N:{n}")).collect();
    sig.sort();
    for op in &u.ops {
        match op {
            LogicalOp::SemanticOp {
                action,
                implementation,
                args,
                fallback,
                ..
            } => {
                let a: Vec<String> = args.iter().map(|(k, v)| format!("{k}={v}")).collect();
                sig.push(format!(
                    "S:{action}.{implementation}:{}:{fallback}",
                    a.join(",")
                ));
            }
            LogicalOp::RunShell { script, env, .. } => {
                let mut e: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
                e.sort();
                sig.push(format!("R:{script}:{}", e.join(",")));
            }
            LogicalOp::DownloadArtifact { name, .. } => sig.push(format!("D:{name}")),
            LogicalOp::TransferArtifact { name, access, .. } => {
                sig.push(format!("T:{name}:{access:?}"))
            }
            LogicalOp::RestoreCache { key } => sig.push(format!("RC:{key}")),
            LogicalOp::SaveCache { key } => sig.push(format!("SC:{key}")),
            LogicalOp::RequestApproval { reason } => sig.push(format!("AP:{reason}")),
            LogicalOp::UploadArtifact { .. } => {}
        }
    }
    sig
}

fn outputs(u: &ExecutionUnit) -> Vec<Ustr> {
    u.ops
        .iter()
        .filter_map(|op| match op {
            LogicalOp::UploadArtifact { name, .. } => Some(*name),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analysis;
    use crate::backends::BackendCapabilities;
    use crate::ir::{ArtifactType, ConsequenceKind, Workflow, WorkflowBuilder, default_objectives};
    use crate::optimize::{OptLevel, OptimizeCtx, optimize};
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

    /// Two identical generators ("make-tool"); each feeds a distinct consumer.
    /// After dedup the survivor feeds both consumers, so the (O3) fusion pass
    /// can't merge it (dependents > 1) — isolating dedup's effect.
    fn dup_wf() -> Workflow {
        let mut b = WorkflowBuilder::new("w");
        let ta = b.artifact("tool_a", ArtifactType::Binary);
        let tb = b.artifact("tool_b", ArtifactType::Binary);
        let o1 = b.artifact("out_1", ArtifactType::Binary);
        let o2 = b.artifact("out_2", ArtifactType::Binary);
        b.shell_action("gen_a", "gen", &[], &[ta], "make-tool");
        b.shell_action("gen_b", "gen", &[], &[tb], "make-tool");
        b.shell_action("use_1", "use1", &[ta], &[o1], "use one");
        b.shell_action("use_2", "use2", &[tb], &[o2], "use two");
        b.actor("r", &["ubuntu-latest"], &[]);
        b.build()
    }

    #[test]
    fn drops_duplicate_and_rewires_consumer() {
        let plan = run(&dup_wf(), OptLevel::O3);
        // gen_b removed; gen_a + use_1 + use_2 remain.
        assert_eq!(
            plan.units.len(),
            3,
            "{:?}",
            plan.units.iter().map(|u| u.action_name).collect::<Vec<_>>()
        );
        let gen_a = plan
            .units
            .iter()
            .find(|u| u.action_name == "gen_a")
            .unwrap();
        let use_2 = plan
            .units
            .iter()
            .find(|u| u.action_name == "use_2")
            .unwrap();
        // use_2 was rewired onto gen_a and its output "tool_a".
        assert!(use_2.needs.contains(&gen_a.id));
        assert!(
            use_2.ops.iter().any(
                |op| matches!(op, LogicalOp::DownloadArtifact { name, .. } if name == "tool_a")
            )
        );
        assert!(plan.optimizations.iter().any(|d| d.pass == "dedup"));
    }

    #[test]
    fn does_not_run_below_o3() {
        let plan = run(&dup_wf(), OptLevel::O2);
        assert_eq!(plan.units.len(), 4);
    }

    #[test]
    fn distinct_computations_not_deduped() {
        let mut b = WorkflowBuilder::new("w");
        let a = b.artifact("a", ArtifactType::Binary);
        let c = b.artifact("c", ArtifactType::Binary);
        b.shell_action("x", "gen", &[], &[a], "make-x");
        b.shell_action("y", "gen", &[], &[c], "make-y"); // different script
        b.actor("r", &["ubuntu-latest"], &[]);
        let plan = run(&b.build(), OptLevel::O3);
        assert_eq!(plan.units.len(), 2);
    }

    #[test]
    fn effectful_duplicates_not_deduped() {
        let mut b = WorkflowBuilder::new("w");
        let a = b.artifact("a", ArtifactType::Binary);
        let c = b.artifact("c", ArtifactType::Binary);
        let x = b.shell_action("x", "deploy", &[], &[a], "ship");
        let y = b.shell_action("y", "deploy", &[], &[c], "ship");
        let e1 = b.consequence("ship1", ConsequenceKind::Deployment, false);
        let e2 = b.consequence("ship2", ConsequenceKind::Deployment, false);
        b.add_consequence_to(x, e1);
        b.add_consequence_to(y, e2);
        b.actor("r", &["ubuntu-latest"], &[]);
        let plan = run(&b.build(), OptLevel::O3);
        assert_eq!(plan.units.len(), 2, "two deploys are not the same effect");
    }
}

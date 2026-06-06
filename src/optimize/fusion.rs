//! Fusion: collapse a producer into its sole consumer when they share a runner,
//! eliminating the artifact transfer between them (the data stays in-process).
//! Consequence-aware: the producer must be pure and neither unit an effect barrier,
//! and fusion preserves op order, so no effect is ever reordered.

use super::{OptLevel, OptimizeCtx, Pass};
use crate::planner::{ExecutionUnit, LogicalOp, OptimizationDecision, Plan};
use std::collections::HashSet;
use ustr::Ustr;

pub struct FusionPass;

impl Pass for FusionPass {
    fn name(&self) -> &str {
        "fusion"
    }
    fn min_level(&self) -> OptLevel {
        OptLevel::O3
    }

    fn run(&self, plan: &mut Plan, ctx: &OptimizeCtx) -> Vec<OptimizationDecision> {
        let mut decisions = Vec::new();
        // Fuse to a fixpoint: each fusion can expose another.
        while let Some((p_idx, c_idx)) = next_fusible(plan, ctx) {
            let producer = plan.units[p_idx].clone();
            let consumer = &mut plan.units[c_idx];
            let shared = shared_artifacts(&producer, consumer);

            // Producer's compute ops (drop uploads of artifacts consumed here),
            // then consumer's ops (drop the matching downloads/transfers).
            let mut ops: Vec<LogicalOp> = producer
                .ops
                .iter()
                .filter(|op| !is_upload_of(op, &shared))
                .cloned()
                .collect();
            ops.extend(consumer.ops.drain(..).filter(|op| !is_pull_of(op, &shared)));
            dedup_input_pulls(&mut ops);
            consumer.ops = ops;

            consumer.needs.retain(|n| *n != producer.id);
            for n in &producer.needs {
                if !consumer.needs.contains(n) {
                    consumer.needs.push(*n);
                }
            }
            merge_unique(&mut consumer.secrets, &producer.secrets);
            merge_unique(&mut consumer.dependencies, &producer.dependencies);
            merge_unique(&mut consumer.toolchains, &producer.toolchains);

            decisions.push(OptimizationDecision {
                pass: "fusion".into(),
                target: consumer.id.to_string(),
                from: format!("{} -> {}", producer.action_name, consumer.action_name),
                to: consumer.action_name.to_string(),
                reason: format!(
                    "fused producer '{}' into sole consumer on shared runner '{}'",
                    producer.action_name, consumer.runner
                ),
            });
            plan.units.remove(p_idx);
        }
        decisions
    }
}

/// First `(producer_idx, consumer_idx)` pair safe to fuse, or `None`.
fn next_fusible(plan: &Plan, ctx: &OptimizeCtx) -> Option<(usize, usize)> {
    for (c_idx, consumer) in plan.units.iter().enumerate() {
        for need in &consumer.needs {
            let Some(p_idx) = plan.units.iter().position(|u| u.id == *need) else {
                continue;
            };
            let producer = &plan.units[p_idx];
            if dependents(plan, producer.id) != 1 {
                continue; // producer feeds others; fusing would duplicate work
            }
            if producer.runner != consumer.runner {
                continue;
            }
            if !ctx.analysis.is_pure(producer.action_id) {
                continue; // never absorb effects/barriers
            }
            if ctx.analysis.is_barrier(consumer.action_id) {
                continue;
            }
            if !caps_superset(consumer, producer) {
                continue;
            }
            return Some((p_idx, c_idx));
        }
    }
    None
}

fn dependents(plan: &Plan, id: Ustr) -> usize {
    plan.units.iter().filter(|u| u.needs.contains(&id)).count()
}

/// Producer outputs that the consumer pulls in (uploaded by P, downloaded by C).
fn shared_artifacts(producer: &ExecutionUnit, consumer: &ExecutionUnit) -> HashSet<Ustr> {
    let produced: HashSet<Ustr> = producer
        .ops
        .iter()
        .filter_map(|op| match op {
            LogicalOp::UploadArtifact { name, .. } => Some(*name),
            _ => None,
        })
        .collect();
    consumer
        .ops
        .iter()
        .filter_map(|op| match op {
            LogicalOp::DownloadArtifact { name, .. } | LogicalOp::TransferArtifact { name, .. }
                if produced.contains(name) =>
            {
                Some(*name)
            }
            _ => None,
        })
        .collect()
}

/// Both halves of a fused unit may pull the same input (e.g. each downloaded
/// `src`). Input acquisition is idempotent within one runner, so keep only the
/// first of each identical checkout/download/transfer. Real work (shell, native,
/// upload, cache) is never deduplicated.
pub(super) fn dedup_input_pulls(ops: &mut Vec<LogicalOp>) {
    let mut seen: HashSet<String> = HashSet::new();
    ops.retain(|op| {
        let key = match op {
            LogicalOp::CheckoutRepo => "checkout".to_string(),
            LogicalOp::DownloadArtifact { name, path } => format!("dl:{name}:{path:?}"),
            LogicalOp::TransferArtifact { name, path, access } => {
                format!("tr:{name}:{path:?}:{access:?}")
            }
            _ => return true,
        };
        seen.insert(key)
    });
}

fn is_upload_of(op: &LogicalOp, shared: &HashSet<Ustr>) -> bool {
    matches!(op, LogicalOp::UploadArtifact { name, .. } if shared.contains(name))
}

fn is_pull_of(op: &LogicalOp, shared: &HashSet<Ustr>) -> bool {
    match op {
        LogicalOp::DownloadArtifact { name, .. } | LogicalOp::TransferArtifact { name, .. } => {
            shared.contains(name)
        }
        _ => false,
    }
}

fn caps_superset(consumer: &ExecutionUnit, producer: &ExecutionUnit) -> bool {
    producer
        .actor_capabilities
        .iter()
        .all(|c| consumer.actor_capabilities.contains(c))
}

fn merge_unique(into: &mut Vec<Ustr>, from: &[Ustr]) {
    for x in from {
        if !into.contains(x) {
            into.push(*x);
        }
    }
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

    /// prep -> build, both pure, same actor: a fusible chain.
    fn chain_wf() -> Workflow {
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::Binary);
        let bin = b.artifact("bin", ArtifactType::Binary);
        let prep = b.shell_action("prep", "prep", &[], &[src], "echo prep");
        let build = b.shell_action("build", "build", &[src], &[bin], "make");
        let a = b.actor("host", &["self-hosted"], &[]);
        b.constrain_actor(prep, a);
        b.constrain_actor(build, a);
        b.build()
    }

    #[test]
    fn fuses_linear_chain_and_drops_transfer() {
        let plan = run(&chain_wf(), OptLevel::O3);
        assert_eq!(plan.units.len(), 1, "prep should be fused into build");
        let unit = &plan.units[0];
        assert_eq!(unit.action_name.as_str(), "build");
        assert!(
            unit.ops.iter().any(
                |op| matches!(op, LogicalOp::RunShell { script, .. } if script == "echo prep")
            )
        );
        // The src transfer is gone (no upload, no download/transfer of "src").
        assert!(plan.access_mode("src").is_none());
        assert!(plan.optimizations.iter().any(|d| d.pass == "fusion"));
    }

    #[test]
    fn fusion_dedups_shared_input_pull() {
        // checkout (runner A) produces src; prep and build (runner H) both consume
        // it. prep's output feeds only build, so prep fuses into build. The fused
        // job must download src once, not once per fused half.
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::Binary);
        let mid = b.artifact("mid", ArtifactType::Binary);
        let bin = b.artifact("bin", ArtifactType::Binary);
        let co = b.shell_action("checkout", "checkout", &[], &[src], "git");
        let prep = b.shell_action("prep", "prep", &[src], &[mid], "echo prep");
        let build = b.shell_action("build", "build", &[src, mid], &[bin], "make");
        let a = b.actor("ci", &["x86_64"], &[]);
        let h = b.actor("host", &["self-hosted"], &[]);
        b.constrain_actor(co, a);
        b.constrain_actor(prep, h);
        b.constrain_actor(build, h);
        let plan = run(&b.build(), OptLevel::O3);
        let fused = plan
            .units
            .iter()
            .find(|u| u.action_name.as_str() == "build")
            .unwrap();
        let src_downloads = fused.ops.iter()
            .filter(|op| matches!(op, LogicalOp::DownloadArtifact { name, .. } if name.as_str() == "src"))
            .count();
        assert_eq!(
            src_downloads, 1,
            "shared input should be downloaded exactly once after fusion"
        );
    }

    #[test]
    fn does_not_run_below_o3() {
        let plan = run(&chain_wf(), OptLevel::O2);
        assert_eq!(plan.units.len(), 2);
    }

    #[test]
    fn different_runners_not_fused() {
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::Binary);
        let bin = b.artifact("bin", ArtifactType::Binary);
        let prep = b.shell_action("prep", "prep", &[], &[src], "echo");
        let build = b.shell_action("build", "build", &[src], &[bin], "make");
        let a = b.actor("a", &["x"], &[]);
        let c = b.actor("c", &["y"], &[]);
        b.constrain_actor(prep, a);
        b.constrain_actor(build, c);
        let plan = run(&b.build(), OptLevel::O3);
        assert_eq!(plan.units.len(), 2);
    }

    #[test]
    fn effectful_producer_not_fused() {
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::Binary);
        let bin = b.artifact("bin", ArtifactType::Binary);
        let prep = b.shell_action("prep", "prep", &[], &[src], "echo");
        let build = b.shell_action("build", "build", &[src], &[bin], "make");
        let eff = b.consequence("ship", ConsequenceKind::Deployment, false);
        b.add_consequence_to(prep, eff); // producer carries a barrier effect
        let a = b.actor("host", &["self-hosted"], &[]);
        b.constrain_actor(prep, a);
        b.constrain_actor(build, a);
        let plan = run(&b.build(), OptLevel::O3);
        assert_eq!(plan.units.len(), 2, "effectful producer must not be fused");
    }

    #[test]
    fn shared_producer_not_fused() {
        // prep feeds two consumers → fusing would duplicate prep.
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::Binary);
        let b1 = b.artifact("b1", ArtifactType::Binary);
        let b2 = b.artifact("b2", ArtifactType::Binary);
        let prep = b.shell_action("prep", "prep", &[], &[src], "echo");
        let x = b.shell_action("x", "build", &[src], &[b1], "make x");
        let y = b.shell_action("y", "build", &[src], &[b2], "make y");
        let a = b.actor("host", &["self-hosted"], &[]);
        b.constrain_actor(prep, a);
        b.constrain_actor(x, a);
        b.constrain_actor(y, a);
        let plan = run(&b.build(), OptLevel::O3);
        assert_eq!(plan.units.len(), 3);
    }
}

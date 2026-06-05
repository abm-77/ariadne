//! Colocation: when a copied artifact's producer and all its consumers already
//! land on the same runner and the backend supports colocation, the bytes never
//! need to leave the host — replace the upload/download copy with a same-host
//! path reference. Always a strict improvement over copy; the copy remains the
//! legal fallback whenever colocation does not hold.

use super::{OptLevel, OptimizeCtx, Pass};
use crate::backends::BackendCapabilities;
use crate::planner::{AccessMode, ExecutionUnit, OptimizationDecision, PhysicalOp, Plan};
use ustr::Ustr;

pub struct ColocationPass;

impl Pass for ColocationPass {
    fn name(&self) -> &str { "colocation" }
    fn min_level(&self) -> OptLevel { OptLevel::O2 }

    fn run(&self, plan: &mut Plan, ctx: &OptimizeCtx) -> Vec<OptimizationDecision> {
        if !ctx.backend_caps.contains(BackendCapabilities::COLOCATION) {
            return Vec::new();
        }
        let mut decisions = Vec::new();
        for name in copied_artifacts(plan) {
            let runners: Vec<Ustr> = plan.units.iter()
                .filter(|u| touches(u, name))
                .map(|u| u.runner)
                .collect();
            let colocated = runners.windows(2).all(|w| w[0] == w[1]);
            if !colocated {
                continue;
            }
            for unit in &mut plan.units {
                let ops = std::mem::take(&mut unit.ops);
                unit.ops = ops.into_iter().filter_map(|op| match op {
                    PhysicalOp::DownloadArtifact { name: n, path } if n == name =>
                        Some(PhysicalOp::TransferArtifact { name: n, path, access: AccessMode::SameHostPath }),
                    PhysicalOp::UploadArtifact { name: n, .. } if n == name => None,
                    other => Some(other),
                }).collect();
            }
            decisions.push(OptimizationDecision {
                pass: "colocation".into(),
                target: name.to_string(),
                from: "copy".into(),
                to: "same_host_path".into(),
                reason: format!("producer and consumers share runner '{}'", runners[0]),
            });
        }
        decisions
    }
}

/// Artifacts currently moved between units by copy (a consumer downloads them).
fn copied_artifacts(plan: &Plan) -> Vec<Ustr> {
    let mut names = Vec::new();
    for unit in &plan.units {
        for op in &unit.ops {
            if let PhysicalOp::DownloadArtifact { name, .. } = op
                && !names.contains(name)
            {
                names.push(*name);
            }
        }
    }
    names
}

fn touches(unit: &ExecutionUnit, name: Ustr) -> bool {
    unit.ops.iter().any(|op| match op {
        PhysicalOp::UploadArtifact { name: n, .. } | PhysicalOp::DownloadArtifact { name: n, .. } => *n == name,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analysis;
    use crate::ir::{default_objectives, ArtifactType, Workflow, WorkflowBuilder};
    use crate::optimize::{optimize, OptLevel, OptimizeCtx};
    use crate::profile::Profile;

    fn run(wf: &Workflow, caps: BackendCapabilities, level: OptLevel) -> Plan {
        let plan = crate::planner::plan(wf).unwrap();
        let profile = Profile::default();
        let analysis = Analysis::of(wf);
        let ctx = OptimizeCtx {
            workflow: wf,
            profile: &profile,
            backend_caps: caps,
            policy: &wf.policies,
            analysis: &analysis,
            objectives: default_objectives(),
            level,
        };
        optimize(plan, &ctx)
    }

    /// Producer and consumer pinned to the same actor.
    fn same_host_wf() -> Workflow {
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::SourceTree);
        let bin = b.artifact("bin", ArtifactType::Binary);
        let prep = b.shell_action("prep", "prep", &[], &[src], "echo");
        let build = b.shell_action("build", "build", &[src], &[bin], "make");
        let actor = b.actor("host", &["self-hosted"], &[]);
        b.constrain_actor(prep, actor);
        b.constrain_actor(build, actor);
        b.build()
    }

    #[test]
    fn colocated_copy_becomes_same_host_path() {
        let plan = run(&same_host_wf(), BackendCapabilities::COLOCATION, OptLevel::O2);
        assert_eq!(plan.access_mode("src"), Some(AccessMode::SameHostPath));
        let prep = plan.units.iter().find(|u| u.action_name == "prep").unwrap();
        assert!(!prep.ops.iter().any(|op| matches!(op, PhysicalOp::UploadArtifact { .. })));
        assert!(plan.optimizations.iter().any(|d| d.to == "same_host_path"));
    }

    #[test]
    fn no_colocation_capability_keeps_copy() {
        let plan = run(&same_host_wf(), BackendCapabilities::empty(), OptLevel::O2);
        assert_eq!(plan.access_mode("src"), Some(AccessMode::Copy));
    }

    #[test]
    fn only_runs_at_o2() {
        let plan = run(&same_host_wf(), BackendCapabilities::COLOCATION, OptLevel::O1);
        assert_eq!(plan.access_mode("src"), Some(AccessMode::Copy));
    }

    #[test]
    fn different_runners_keep_copy() {
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::SourceTree);
        let bin = b.artifact("bin", ArtifactType::Binary);
        let prep = b.shell_action("prep", "prep", &[], &[src], "echo");
        let build = b.shell_action("build", "build", &[src], &[bin], "make");
        let a = b.actor("a", &["x"], &[]);
        let c = b.actor("c", &["y"], &[]);
        b.constrain_actor(prep, a);
        b.constrain_actor(build, c);
        let plan = run(&b.build(), BackendCapabilities::COLOCATION, OptLevel::O2);
        assert_eq!(plan.access_mode("src"), Some(AccessMode::Copy));
    }
}

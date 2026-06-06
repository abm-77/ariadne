//! Placement optimization: upgrade copy (upload/download) to mount when a
//! shared placement is declared and the backend + all involved actors support
//! mounts. Otherwise keep the copy fallback and warn — optimization is never
//! required for correctness.

use super::{OptLevel, OptimizeCtx, Pass};
use crate::backends::BackendCapabilities;
use crate::diagnostics::{DiagCode, Diagnostic};
use crate::ir::PlacementStrategy;
use crate::planner::{
    AccessMode, ExecutionUnit, LogicalOp, MOUNT_CAPABILITY, OptimizationDecision, Plan,
};
use ustr::Ustr;

pub struct PlacementPass;

impl Pass for PlacementPass {
    fn name(&self) -> &str {
        "placement"
    }
    fn min_level(&self) -> OptLevel {
        OptLevel::O1
    }

    fn run(&self, plan: &mut Plan, ctx: &OptimizeCtx) -> Vec<OptimizationDecision> {
        let mut decisions = Vec::new();

        // Artifacts the workflow asked to place on shared/local storage — the
        // signal that the user wants something better than copy.
        let shared: Vec<Ustr> = ctx
            .workflow
            .placements
            .iter()
            .filter(|p| {
                matches!(
                    p.strategy,
                    PlacementStrategy::SharedVolume { .. } | PlacementStrategy::LocalPath { .. }
                )
            })
            .map(|p| ctx.workflow.artifact(p.artifact).name)
            .collect();

        for name in shared {
            if plan.access_mode(&name) != Some(AccessMode::Copy) {
                continue; // not a cross-unit copy (nothing to upgrade)
            }
            let mountable = ctx.backend_caps.contains(BackendCapabilities::MOUNTS)
                && plan.units.iter().filter(|u| touches(u, name)).all(|u| {
                    u.actor_capabilities
                        .iter()
                        .any(|c| c.as_str() == MOUNT_CAPABILITY)
                });

            if mountable {
                upgrade_to_mount(plan, name);
                decisions.push(OptimizationDecision {
                    pass: "placement".into(),
                    target: name.to_string(),
                    from: "copy".into(),
                    to: "mount_read_only".into(),
                    reason: "shared placement; backend and actors support mounts".into(),
                });
            } else {
                plan.diagnostics.push(Diagnostic::warning(
                    DiagCode::FallbackPlacementSelected,
                    format!("Artifact '{name}' uses copy fallback: mount unavailable (backend or actors lack mounts)"),
                ));
                decisions.push(OptimizationDecision {
                    pass: "placement".into(),
                    target: name.to_string(),
                    from: "copy".into(),
                    to: "copy".into(),
                    reason: "mount unavailable; kept copy fallback".into(),
                });
            }
        }
        decisions
    }
}

fn touches(unit: &ExecutionUnit, name: Ustr) -> bool {
    unit.ops.iter().any(|op| match op {
        LogicalOp::UploadArtifact { name: n, .. } | LogicalOp::DownloadArtifact { name: n, .. } => {
            *n == name
        }
        _ => false,
    })
}

/// Consumers mount read-only; the producer no longer needs to upload.
fn upgrade_to_mount(plan: &mut Plan, name: Ustr) {
    for unit in &mut plan.units {
        let ops = std::mem::take(&mut unit.ops);
        unit.ops = ops
            .into_iter()
            .filter_map(|op| match op {
                LogicalOp::DownloadArtifact { name: n, path } if n == name => {
                    Some(LogicalOp::TransferArtifact {
                        name: n,
                        path,
                        access: AccessMode::MountReadOnly,
                    })
                }
                LogicalOp::UploadArtifact { name: n, .. } if n == name => None,
                other => Some(other),
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analysis;
    use crate::ir::{ArtifactType, PlacementStrategy, WorkflowBuilder, default_objectives};
    use crate::optimize::optimize;
    use crate::profile::Profile;

    fn shared_wf() -> crate::ir::Workflow {
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::Binary);
        let bin = b.artifact("bin", ArtifactType::Binary);
        let prep = b.shell_action("prep", "prep", &[], &[src], "echo");
        let build = b.shell_action("build", "build", &[src], &[bin], "make");
        let actor = b.actor("big", &["self-hosted"], &["mount"]);
        b.constrain_actor(prep, actor);
        b.constrain_actor(build, actor);
        b.place(
            src,
            PlacementStrategy::SharedVolume {
                path: "/vol".into(),
            },
        );
        b.build()
    }

    fn run(wf: &crate::ir::Workflow, caps: BackendCapabilities) -> Plan {
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
            level: OptLevel::O1,
        };
        optimize(plan, &ctx)
    }

    #[test]
    fn baseline_is_copy() {
        let plan = crate::planner::plan(&shared_wf()).unwrap();
        assert_eq!(plan.access_mode("src"), Some(AccessMode::Copy));
    }

    #[test]
    fn upgrades_to_mount_when_available() {
        let plan = run(
            &shared_wf(),
            BackendCapabilities::MOUNTS | BackendCapabilities::COLOCATION,
        );
        assert_eq!(plan.access_mode("src"), Some(AccessMode::MountReadOnly));
        // The producer's upload was dropped (mounted, not uploaded).
        let prep = plan.units.iter().find(|u| u.action_name == "prep").unwrap();
        assert!(
            !prep
                .ops
                .iter()
                .any(|op| matches!(op, LogicalOp::UploadArtifact { .. }))
        );
        assert!(plan.optimizations.iter().any(|d| d.to == "mount_read_only"));
    }

    #[test]
    fn falls_back_to_copy_with_warning_when_unavailable() {
        // GitHub-like backend: no mounts.
        let plan = run(&shared_wf(), BackendCapabilities::CACHE);
        assert_eq!(plan.access_mode("src"), Some(AccessMode::Copy));
        assert!(
            plan.diagnostics
                .iter()
                .any(|d| d.to_string().contains("mount unavailable"))
        );
    }

    #[test]
    fn no_placement_means_no_warning() {
        // Same shape but no declared placement → plain copy, no "fallback" noise.
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::Binary);
        let bin = b.artifact("bin", ArtifactType::Binary);
        b.shell_action("prep", "prep", &[], &[src], "echo");
        b.shell_action("build", "build", &[src], &[bin], "make");
        b.actor("l", &["ubuntu-latest"], &[]);
        let plan = run(&b.build(), BackendCapabilities::empty());
        assert_eq!(plan.access_mode("src"), Some(AccessMode::Copy));
        assert!(plan.diagnostics.is_empty());
    }
}

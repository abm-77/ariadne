use crate::diagnostics::{DiagCode, Diagnostic};
use serde::{Deserialize, Serialize};
use ustr::Ustr;
use std::collections::{HashMap, VecDeque};
use crate::ir::{Action, ActionId, Actor, ActorConstraint, Workflow};

pub use crate::ir::EffectKind;

/// Well-known capability the planner reasons about for placement: an actor with
/// it can mount shared volumes, enabling mount transfers instead of copies.
/// Capabilities are otherwise open strings defined by backends/operators.
pub const MOUNT_CAPABILITY: &str = "mount";

/// An effect lowered onto an execution unit. Effects never run during `loom
/// test`; they are recorded so a test can assert which would fire or be gated.
#[derive(Debug, Clone)]
pub struct EffectInfo {
    pub name: Ustr,
    pub kind: EffectKind,
    pub requires_approval: bool,
}

/// The triggering event a plan is built for. Planning is event-aware: the event
/// decides which effects fire vs are gated, and whether secrets are available.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventContext {
    Push { branch: String },
    PullRequest { fork: bool },
    Tag { name: String },
}

impl Default for EventContext {
    fn default() -> Self { EventContext::Push { branch: "main".into() } }
}

impl EventContext {
    /// Fork pull requests never receive secrets, the core security property.
    pub fn secrets_available(&self) -> bool {
        !matches!(self, EventContext::PullRequest { fork: true })
    }

    /// Privileged effects do not fire on pull requests.
    pub fn effects_allowed(&self) -> bool {
        !matches!(self, EventContext::PullRequest { .. })
    }
}

/// An effect fires only if the event permits it and it needs no approval;
/// otherwise it is gated. Effects are never executed either way.
pub fn partition_effects(effects: &[EffectInfo], event: &EventContext) -> (Vec<EffectInfo>, Vec<EffectInfo>) {
    let mut fired = Vec::new();
    let mut gated = Vec::new();
    for e in effects {
        if event.effects_allowed() && !e.requires_approval {
            fired.push(e.clone());
        } else {
            gated.push(e.clone());
        }
    }
    (fired, gated)
}

/// Backend-neutral physical operations.
/// The planner emits these; instruction selection maps them to backend-specific steps.
#[derive(Debug, Clone)]
pub enum PhysicalOp {
    CheckoutRepo,
    RunShell {
        label: Ustr,
        script: String,
        env: HashMap<String, String>,
    },
    UploadArtifact { name: Ustr, path: Option<String> },
    DownloadArtifact { name: Ustr, path: Option<String> },
    TransferArtifact { name: Ustr, path: Option<String>, access: AccessMode },
    RestoreCache { key: Ustr },
    SaveCache { key: Ustr },
    RequestApproval { reason: String },
}

impl PhysicalOp {
    /// Stable variant name, used for instruction matching and selection.
    pub fn name(&self) -> &'static str {
        match self {
            PhysicalOp::CheckoutRepo => "CheckoutRepo",
            PhysicalOp::RunShell { .. } => "RunShell",
            PhysicalOp::UploadArtifact { .. } => "UploadArtifact",
            PhysicalOp::DownloadArtifact { .. } => "DownloadArtifact",
            PhysicalOp::TransferArtifact { .. } => "TransferArtifact",
            PhysicalOp::RestoreCache { .. } => "RestoreCache",
            PhysicalOp::SaveCache { .. } => "SaveCache",
            PhysicalOp::RequestApproval { .. } => "RequestApproval",
        }
    }
}

/// How an artifact is made available to a consuming unit. `Copy` (upload/
/// download) is the always-legal baseline; the optimizer upgrades to richer
/// modes when the backend and actors support them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    Copy,
    MountReadOnly,
    MountReadWrite,
    Stream,
    SameHostPath,
    OciLayer,
}

#[derive(Debug, Clone)]
pub struct ExecutionUnit {
    pub id: Ustr,
    pub action_id: ActionId,
    pub action_name: Ustr,
    pub needs: Vec<Ustr>,
    pub ops: Vec<PhysicalOp>,
    pub runner: Ustr,
    /// Secrets this unit's action declared. The executor/backend decides how to
    /// wire them; under `loom test` they are spoofed into the container env.
    pub secrets: Vec<Ustr>,
    /// Effects this unit's action declared. Recorded, never executed.
    pub effects: Vec<EffectInfo>,
    /// Effects that would fire under the plan's event (subset of `effects`).
    pub effects_fired: Vec<EffectInfo>,
    /// Effects gated by the event or by required approval (subset of `effects`).
    pub effects_gated: Vec<EffectInfo>,
    /// Whether this unit's secrets are available under the plan's event.
    pub secrets_available: bool,
    /// Capabilities the unit's resolved actor provides (open strings).
    pub actor_capabilities: Vec<Ustr>,
}

/// A recorded optimization decision, filled by the optimize phase and surfaced
/// by `loom explain`. Lives here (plan metadata) so `Plan` can hold it without a
/// planner -> optimize dependency.
#[derive(Debug, Clone)]
pub struct OptimizationDecision {
    pub pass: String,
    pub target: String,
    pub from: String,
    pub to: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub workflow_name: Ustr,
    pub max_parallel_jobs: Option<usize>,
    pub units: Vec<ExecutionUnit>,
    pub diagnostics: Vec<Diagnostic>,
    /// The event this plan was built for.
    pub event: EventContext,
    /// Optimization decisions applied to this plan (empty for a baseline plan).
    pub optimizations: Vec<OptimizationDecision>,
}

impl Plan {
    /// The access mode chosen for a cross-unit artifact (by name), derived from
    /// the lowered ops. `None` if the artifact is not transferred between units.
    pub fn access_mode(&self, artifact: &str) -> Option<AccessMode> {
        for unit in &self.units {
            for op in &unit.ops {
                match op {
                    PhysicalOp::TransferArtifact { name, access, .. } if name == artifact => return Some(*access),
                    PhysicalOp::DownloadArtifact { name, .. } if name == artifact => return Some(AccessMode::Copy),
                    _ => {}
                }
            }
        }
        None
    }

    /// Maximum number of units that can run concurrently, derived from the
    /// dependency (`needs`) DAG via longest-path levels. Each level is an
    /// antichain, so the widest level bounds real concurrency. Assumes units are
    /// in topological order (as the planner emits them).
    pub fn max_concurrency(&self) -> usize {
        let mut level: HashMap<Ustr, usize> = HashMap::new();
        let mut width: HashMap<usize, usize> = HashMap::new();
        for u in &self.units {
            let l = u.needs.iter().filter_map(|n| level.get(n)).max().map_or(0, |m| m + 1);
            level.insert(u.id, l);
            *width.entry(l).or_default() += 1;
        }
        width.into_values().max().unwrap_or(0)
    }
}

/// Plan under the default push event.
pub fn plan(workflow: &Workflow) -> Result<Plan, Vec<Diagnostic>> {
    plan_for(workflow, &EventContext::default())
}

/// Plan for a specific triggering event. The event reshapes the plan: effects
/// are partitioned into fired/gated and secret availability is decided here.
pub fn plan_for(workflow: &Workflow, event: &EventContext) -> Result<Plan, Vec<Diagnostic>> {
    let topo = topo_sort(workflow)?;

    let default_runner: Ustr = workflow.actors.first()
        .and_then(|a| a.labels.first().copied())
        .unwrap_or_else(|| Ustr::from("ubuntu-latest"));

    let mut unit_id_for_action: HashMap<ActionId, Ustr> = HashMap::new();
    let mut units: Vec<ExecutionUnit> = Vec::new();
    let diagnostics: Vec<Diagnostic> = Vec::new();

    for action_id in topo {
        let action = workflow.action(action_id);
        let unit_id = Ustr::from(sanitize_id(&action.name).as_str());
        unit_id_for_action.insert(action_id, unit_id);

        let mut needs: Vec<Ustr> = action.inputs.iter()
            .filter_map(|&art_id| workflow.artifact(art_id).producer)
            .filter(|&pred| pred != action_id)
            .filter_map(|pred| unit_id_for_action.get(&pred).copied())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        needs.sort();

        let mut ops: Vec<PhysicalOp> = Vec::new();

        // Baseline placement is always copy (upload/download) — the legal
        // fallback. The placement optimization pass upgrades to mount/colocation
        // when a backend and actors support it.
        for &in_id in &action.inputs {
            let art = workflow.artifact(in_id);
            if art.producer.is_none() || art.producer == Some(action_id) {
                continue;
            }
            ops.push(PhysicalOp::DownloadArtifact { name: art.name, path: art.path.clone() });
        }

        match action.op.as_str() {
            "checkout" => ops.push(PhysicalOp::CheckoutRepo),
            _ => {
                if let Some(ref sh) = action.shell {
                    ops.push(PhysicalOp::RunShell {
                        label: action.name,
                        script: sh.script.clone(),
                        env: sh.env.clone(),
                    });
                }
            }
        }

        for &out_id in &action.outputs {
            let out = workflow.artifact(out_id);
            ops.push(PhysicalOp::UploadArtifact { name: out.name, path: out.path.clone() });
        }

        let effects: Vec<EffectInfo> = action.effects.iter().map(|&eid| {
            let e = workflow.effect(eid);
            EffectInfo { name: e.name, kind: e.kind.clone(), requires_approval: e.requires_approval }
        }).collect();
        let (effects_fired, effects_gated) = partition_effects(&effects, event);

        let actor = actor_for(action, workflow);
        let runner = actor.and_then(|a| a.labels.first().copied())
            .unwrap_or(default_runner);
        let actor_capabilities = actor.map(|a| a.capabilities.clone()).unwrap_or_default();

        units.push(ExecutionUnit {
            id: unit_id,
            action_id,
            action_name: action.name,
            needs,
            ops,
            runner,
            secrets: action.secrets.clone(),
            effects,
            effects_fired,
            effects_gated,
            secrets_available: event.secrets_available(),
            actor_capabilities,
        });
    }

    Ok(Plan {
        workflow_name: workflow.name,
        max_parallel_jobs: workflow.policies.max_parallel_jobs,
        units,
        diagnostics,
        event: event.clone(),
        optimizations: Vec::new(),
    })
}

fn sanitize_id(name: &str) -> String {
    name.to_lowercase().replace([' ', '_'], "-")
}

pub(crate) fn actor_for<'a>(action: &Action, workflow: &'a Workflow) -> Option<&'a Actor> {
    for c in &action.actor_constraints {
        match c {
            ActorConstraint::Specific(id) => return Some(workflow.actor(*id)),
            ActorConstraint::Label(label) => {
                if let Some(a) = workflow.actors.iter().find(|a| a.labels.iter().any(|l| l == label)) {
                    return Some(a);
                }
            }
        }
    }
    workflow.actors.first()
}

fn topo_sort(workflow: &Workflow) -> Result<Vec<ActionId>, Vec<Diagnostic>> {
    let n = workflow.actions.len();
    let mut in_degree: Vec<usize> = vec![0; n];
    let mut rdeps: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (idx, action) in workflow.actions.iter().enumerate() {
        let preds: std::collections::HashSet<usize> = action.inputs.iter()
            .filter_map(|&art_id| workflow.artifact(art_id).producer)
            .filter(|&p| p.idx() != idx)
            .map(|p| p.idx())
            .collect();
        in_degree[idx] = preds.len();
        for pred in preds {
            rdeps[pred].push(idx);
        }
    }

    let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut initial: Vec<usize> = queue.drain(..).collect();
    initial.sort();
    queue.extend(initial);

    let mut order: Vec<ActionId> = Vec::with_capacity(n);

    while let Some(idx) = queue.pop_front() {
        order.push(ActionId(idx as u32));
        let mut newly_ready: Vec<usize> = Vec::new();
        for &dep in &rdeps[idx] {
            in_degree[dep] -= 1;
            if in_degree[dep] == 0 {
                newly_ready.push(dep);
            }
        }
        newly_ready.sort();
        queue.extend(newly_ready);
    }

    if order.len() != n {
        return Err(vec![Diagnostic::error(
            DiagCode::CycleDetected,
            "Cycle detected in action dependency graph",
        )]);
    }

    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::*;

    fn simple_workflow() -> Workflow {
        let mut b = WorkflowBuilder::new("Simple Build and Test");
        let src = b.artifact("source", ArtifactType::SourceTree);
        let bin = b.artifact("binary", ArtifactType::Binary);
        let rep = b.artifact("test-report", ArtifactType::TestReport);
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        b.shell_action("build", "build", &[src], &[bin], "cargo build --release");
        b.shell_action("test", "test", &[bin], &[rep], "cargo test");
        b.actor("github-ubuntu", &["ubuntu-latest"], &[]);
        b.max_parallel_jobs(2);
        b.build()
    }

    #[test]
    fn producer_ordered_before_consumer() {
        let plan = plan(&simple_workflow()).unwrap();
        let pos = |name: &str| plan.units.iter().position(|u| u.action_name == name).unwrap();
        assert!(pos("checkout") < pos("build"));
        assert!(pos("build") < pos("test"));
    }

    #[test]
    fn baseline_emits_no_placement_warnings() {
        // The baseline is pure copy/upload-download: always legal, never a
        // "fallback" — placement upgrades (and any fallback warning) belong to
        // the optimization phase, not the planner.
        let plan = plan(&simple_workflow()).unwrap();
        assert!(plan.diagnostics.iter().all(|d| d.code != DiagCode::FallbackPlacementSelected));
    }

    #[test]
    fn policy_propagated_to_plan() {
        assert_eq!(plan(&simple_workflow()).unwrap().max_parallel_jobs, Some(2));
    }

    #[test]
    fn deployment_effect_preserved_on_action() {
        let mut wf = simple_workflow();
        let eff_id = EffectId(0);
        wf.effects.push(Effect { name: "deploy".into(), kind: EffectKind::Deployment, requires_approval: true });
        wf.actions[2].effects.push(eff_id);
        let p = plan(&wf).unwrap();
        let test_unit = p.units.iter().find(|u| u.action_name == "test").unwrap();
        assert!(wf.action(test_unit.action_id).effects.contains(&eff_id));
    }

    #[test]
    fn checkout_op_emits_checkout_repo_physical_op() {
        let plan = plan(&simple_workflow()).unwrap();
        let checkout = plan.units.iter().find(|u| u.action_name == "checkout").unwrap();
        assert!(checkout.ops.iter().any(|op| matches!(op, PhysicalOp::CheckoutRepo)));
    }

    #[test]
    fn shell_op_emits_run_shell_physical_op() {
        let plan = plan(&simple_workflow()).unwrap();
        let build = plan.units.iter().find(|u| u.action_name == "build").unwrap();
        assert!(build.ops.iter().any(|op| matches!(op, PhysicalOp::RunShell { .. })));
    }

    #[test]
    fn cross_unit_artifact_uses_copy_without_placement() {
        let plan = plan(&simple_workflow()).unwrap();
        let build = plan.units.iter().find(|u| u.action_name == "build").unwrap();
        assert!(build.ops.iter().any(|op| matches!(op, PhysicalOp::DownloadArtifact { .. })));
        assert!(!build.ops.iter().any(|op| matches!(op, PhysicalOp::TransferArtifact { .. })));
    }

    #[test]
    fn baseline_copies_even_with_shared_placement() {
        // A declared shared placement does NOT change the baseline plan; the
        // mount upgrade is the placement optimization pass's job. The planner
        // always produces the correct copy baseline.
        let mut b = WorkflowBuilder::new("mounted");
        let src = b.artifact("source", ArtifactType::SourceTree);
        let bin = b.artifact("binary", ArtifactType::Binary);
        let checkout = b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        let build = b.shell_action("build", "build", &[src], &[bin], "make");
        let gpu = b.actor("big", &["self-hosted"], &["mount"]);
        b.constrain_actor(checkout, gpu);
        b.constrain_actor(build, gpu);
        b.place(src, PlacementStrategy::SharedVolume { path: "/vol".into() });
        let plan = plan(&b.build()).unwrap();
        let build_unit = plan.units.iter().find(|u| u.action_name == "build").unwrap();
        assert!(build_unit.ops.iter().any(|op| matches!(op, PhysicalOp::DownloadArtifact { .. })));
        assert!(!build_unit.ops.iter().any(|op| matches!(op, PhysicalOp::TransferArtifact { .. })));
    }

    #[test]
    fn effects_lowered_onto_unit() {
        let mut wf = simple_workflow();
        wf.effects.push(Effect { name: "deploy".into(), kind: EffectKind::Deployment, requires_approval: true });
        wf.actions[2].effects.push(EffectId(0));
        let plan = plan(&wf).unwrap();
        let test_unit = plan.units.iter().find(|u| u.action_name == "test").unwrap();
        assert_eq!(test_unit.effects.len(), 1);
        assert_eq!(test_unit.effects[0].name, "deploy");
        assert!(test_unit.effects[0].requires_approval);
    }

    fn deploy_workflow(approval: bool) -> Workflow {
        let mut b = WorkflowBuilder::new("w");
        let bin = b.artifact("bin", ArtifactType::Binary);
        let a = b.shell_action("deploy", "deploy", &[], &[bin], "x");
        b.add_secrets(a, &["TOKEN"]);
        let e = b.effect("ship", EffectKind::Deployment, approval);
        b.add_effect_to(a, e);
        b.actor("l", &["ubuntu-latest"], &[]);
        b.build()
    }

    #[test]
    fn effect_fires_on_push_gated_on_pr() {
        let wf = deploy_workflow(false);
        let push = plan_for(&wf, &EventContext::Push { branch: "main".into() }).unwrap();
        assert!(push.units[0].effects_fired.iter().any(|e| e.name == "ship"));
        assert!(push.units[0].effects_gated.is_empty());

        let pr = plan_for(&wf, &EventContext::PullRequest { fork: false }).unwrap();
        assert!(pr.units[0].effects_gated.iter().any(|e| e.name == "ship"));
        assert!(pr.units[0].effects_fired.is_empty());
    }

    #[test]
    fn approval_effect_gated_even_on_push() {
        let push = plan_for(&deploy_workflow(true), &EventContext::default()).unwrap();
        assert!(push.units[0].effects_gated.iter().any(|e| e.name == "ship"));
        assert!(push.units[0].effects_fired.is_empty());
    }

    #[test]
    fn secrets_available_depends_on_event() {
        let wf = deploy_workflow(false);
        assert!(plan_for(&wf, &EventContext::default()).unwrap().units[0].secrets_available);
        assert!(plan_for(&wf, &EventContext::PullRequest { fork: true }).unwrap().units[0].secrets_available == false);
    }

    #[test]
    fn actor_capabilities_recorded_on_unit() {
        let mut b = WorkflowBuilder::new("w");
        let bin = b.artifact("bin", ArtifactType::Binary);
        let a = b.shell_action("build", "build", &[], &[bin], "make");
        let gpu = b.actor("gpu-box", &["self-hosted"], &["gpu", "cuda"]);
        b.constrain_actor(a, gpu);
        let plan = plan(&b.build()).unwrap();
        assert_eq!(plan.units[0].actor_capabilities, vec!["gpu".to_string(), "cuda".to_string()]);
    }

    #[test]
    fn plan_records_its_event() {
        let plan = plan_for(&deploy_workflow(false), &EventContext::Tag { name: "v1".into() }).unwrap();
        assert!(matches!(plan.event, EventContext::Tag { .. }));
    }

    #[test]
    fn artifact_path_lowered_into_ops() {
        let mut b = WorkflowBuilder::new("w");
        let bin = b.artifact_at("bin", ArtifactType::Binary, "target/app");
        b.shell_action("build", "build", &[], &[bin], "make");
        b.actor("l", &["ubuntu-latest"], &[]);
        let plan = plan(&b.build()).unwrap();
        assert!(plan.units[0].ops.iter().any(|op|
            matches!(op, PhysicalOp::UploadArtifact { path: Some(p), .. } if p == "target/app")));
    }

    #[test]
    fn cycle_returns_error() {
        let mut b = WorkflowBuilder::new("cyclic");
        let x = b.artifact("x", ArtifactType::Binary);
        let y = b.artifact("y", ArtifactType::Binary);
        b.action("a", "a", &[y], &[x]);
        b.action("b", "b", &[x], &[y]);
        assert!(plan(&b.build()).is_err());
    }
}

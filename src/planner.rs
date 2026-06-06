use crate::diagnostics::{DiagCode, Diagnostic};
use crate::ir::{self, ActionCall, ActionCallId, Actor, ActorConstraint, ShellAction, Workflow};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use ustr::Ustr;

pub use crate::ir::ConsequenceKind;

/// Well-known capability the planner reasons about for placement: an actor with
/// it can mount shared volumes, enabling mount transfers instead of copies.
/// Capabilities are otherwise open strings defined by backends/operators.
pub const MOUNT_CAPABILITY: &str = "mount";

/// An effect lowered onto an execution unit. Effects never run during `loom
/// test`; they are recorded so a test can assert which would fire or be gated.
#[derive(Debug, Clone)]
pub struct ConsequenceInfo {
    pub name: Ustr,
    pub kind: ConsequenceKind,
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
    fn default() -> Self {
        EventContext::Push {
            branch: "main".into(),
        }
    }
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
pub fn partition_consequences(
    consequences: &[ConsequenceInfo],
    event: &EventContext,
) -> (Vec<ConsequenceInfo>, Vec<ConsequenceInfo>) {
    let mut fired = Vec::new();
    let mut gated = Vec::new();
    for e in consequences {
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
pub enum LogicalOp {
    CheckoutRepo,
    RunShell {
        label: Ustr,
        script: String,
        env: HashMap<String, String>,
    },
    UploadArtifact {
        name: Ustr,
        path: Option<String>,
        lifetime: Option<String>,
    },
    DownloadArtifact {
        name: Ustr,
        path: Option<String>,
    },
    TransferArtifact {
        name: Ustr,
        path: Option<String>,
        access: AccessMode,
    },
    RestoreCache {
        key: Ustr,
    },
    SaveCache {
        key: Ustr,
    },
    RequestApproval {
        reason: String,
    },
    /// A portable semantic instruction. Backends may upgrade it to a native
    /// step (e.g. a GitHub `uses:` action) via their catalogue when the
    /// inventory permits; otherwise `fallback` is run as a shell command. This
    /// keeps the plan backend-agnostic while allowing emit-time native steps.
    Native {
        id: Ustr,
        args: BTreeMap<String, String>,
        fallback: String,
    },
}

impl LogicalOp {
    /// Stable variant name, used for instruction matching and selection.
    pub fn name(&self) -> &'static str {
        match self {
            LogicalOp::CheckoutRepo => "CheckoutRepo",
            LogicalOp::RunShell { .. } => "RunShell",
            LogicalOp::UploadArtifact { .. } => "UploadArtifact",
            LogicalOp::DownloadArtifact { .. } => "DownloadArtifact",
            LogicalOp::TransferArtifact { .. } => "TransferArtifact",
            LogicalOp::RestoreCache { .. } => "RestoreCache",
            LogicalOp::SaveCache { .. } => "SaveCache",
            LogicalOp::RequestApproval { .. } => "RequestApproval",
            LogicalOp::Native { .. } => "Native",
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
    pub action_id: ActionCallId,
    pub action_name: Ustr,
    pub needs: Vec<Ustr>,
    pub ops: Vec<LogicalOp>,
    pub runner: Ustr,
    /// Secrets this unit's action declared. The executor/backend decides how to
    /// wire them; under `loom test` they are spoofed into the container env.
    pub secrets: Vec<Ustr>,
    /// Effects this unit's action declared. Recorded, never executed.
    pub consequences: Vec<ConsequenceInfo>,
    /// Effects that would fire under the plan's event (subset of `effects`).
    pub consequences_fired: Vec<ConsequenceInfo>,
    /// Effects gated by the event or by required approval (subset of `effects`).
    pub consequences_gated: Vec<ConsequenceInfo>,
    /// Whether this unit's secrets are available under the plan's event.
    pub secrets_available: bool,
    /// Capabilities the unit's resolved actor provides (open strings).
    pub actor_capabilities: Vec<Ustr>,
    /// Maximum execution duration for this unit (e.g. "30m"), if declared.
    pub timeout: Option<String>,
    /// Action-level concurrency control, if declared.
    pub coordination: Option<ir::Coordination>,
    /// Names of tools the unit's actions need, referencing `Plan::dependencies`
    /// for the install command. Backends may install them on job start when the
    /// workflow opts in; otherwise the env provides them.
    pub dependencies: Vec<Ustr>,
    /// Language toolchains the unit needs (e.g. "rust", "python"), referencing
    /// `Plan::toolchains` for the version. Backends provision them per job.
    pub toolchains: Vec<Ustr>,
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
    /// Instruction-selection capabilities derived from the inventory's available
    /// implementations (e.g. "impl.pypa-publish-action"). Backends use these to
    /// decide whether to upgrade a `Native` op to a native step at emit time.
    pub impl_capabilities: Vec<String>,
    /// How the workflow is entered. Backends emit their native trigger block
    /// from this; empty means the backend's default trigger.
    pub triggers: Vec<ir::Trigger>,
    /// Workflow-level concurrency control, if declared.
    pub coordination: Option<ir::Coordination>,
    /// Shared dependency table: tool name -> resolved install command (from the
    /// `package.install` lowering). Units reference it by tool name
    /// (`ExecutionUnit::dependencies`) so a tool used by many jobs is stored once.
    pub dependencies: std::collections::BTreeMap<String, String>,
    /// Shared toolchain table: toolchain name -> version/channel (from the
    /// inventory's `.use`). Units reference it by name (`ExecutionUnit::toolchains`).
    pub toolchains: std::collections::BTreeMap<String, Option<String>>,
    /// Whether backends should install each unit's dependencies on job start.
    pub install_dependencies: bool,
}

impl Plan {
    /// The access mode chosen for a cross-unit artifact (by name), derived from
    /// the lowered ops. `None` if the artifact is not transferred between units.
    pub fn access_mode(&self, artifact: &str) -> Option<AccessMode> {
        for unit in &self.units {
            for op in &unit.ops {
                match op {
                    LogicalOp::TransferArtifact { name, access, .. } if name == artifact => {
                        return Some(*access);
                    }
                    LogicalOp::DownloadArtifact { name, .. } if name == artifact => {
                        return Some(AccessMode::Copy);
                    }
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
            let l = u
                .needs
                .iter()
                .filter_map(|n| level.get(n))
                .max()
                .map_or(0, |m| m + 1);
            level.insert(u.id, l);
            *width.entry(l).or_default() += 1;
        }
        width.into_values().max().unwrap_or(0)
    }
}

fn impl_to_shell(i: &ir::Implementation) -> Option<ShellAction> {
    match i {
        ir::Implementation::Shell { run, env, capture } => Some(ShellAction {
            script: run.clone(),
            env: env.clone(),
            capture: capture.clone(),
        }),
        _ => None,
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

    let default_runner: Ustr = workflow
        .actors()
        .first()
        .and_then(|a| a.labels.first().copied())
        .unwrap_or_else(|| Ustr::from("ubuntu-latest"));

    let mut unit_id_for_action: HashMap<ActionCallId, Ustr> = HashMap::new();
    let mut units: Vec<ExecutionUnit> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let lowering_registry = crate::lowering::Registry::builtin();
    // Shared dependency table, populated as lowerings are selected; units only
    // hold tool names that reference it. Each tool resolves (once) to its install
    // command via the `package.install` lowerings, so the manager (pip/apt/...)
    // is chosen by the same selection machinery as any action.
    let dep_store = crate::dependency::DependencyRegistry::builtin();
    let mut dep_table: BTreeMap<String, String> = BTreeMap::new();
    // Toolchains needed across the plan, with versions read from the inventory.
    let mut tc_table: BTreeMap<String, Option<String>> = BTreeMap::new();
    let inv_version = |tc: &str| -> Option<String> {
        workflow.inventory.as_ref().and_then(|i| {
            i.implementations
                .iter()
                .find(|m| m.id.as_str() == tc)
                .and_then(|m| m.version.map(|v| v.to_string()))
        })
    };

    let is_source = |art_id: ir::ArtifactId| {
        matches!(workflow.artifact(art_id).ty, ir::ArtifactType::SourceTree)
    };

    for action_id in topo {
        let action = workflow.action_call(action_id);

        let op_def = workflow.find_action_def(action.action.as_str());
        let is_checkout = action.action.as_str() == "checkout"
            || action.action.as_str() == "scm.checkout"
            || op_def.is_some_and(|d| {
                d.implementations
                    .iter()
                    .any(|i| matches!(i, ir::Implementation::Checkout))
            });

        // A pure source-acquisition action (checkout producing only a SourceTree)
        // is not its own job: SourceTree is ambient, reacquired per job via
        // checkout rather than shipped as an artifact. Skip it; consumers
        // self-checkout below. This is the always-correct realization — a working
        // tree cannot be meaningfully copied as a named artifact, but checkout
        // reproduces it anywhere.
        if is_checkout && action.outputs.iter().all(|&o| is_source(o)) {
            continue;
        }

        let unit_id = Ustr::from(sanitize_id(&action.name).as_str());
        unit_id_for_action.insert(action_id, unit_id);

        // Depend only on producers of non-source inputs; source is self-acquired.
        // Explicit `after` edges add ordering with no data flow (e.g. gates).
        let mut need_set: std::collections::HashSet<Ustr> = action
            .inputs
            .iter()
            .filter(|&&art_id| !is_source(art_id))
            .filter_map(|&art_id| workflow.artifact(art_id).producer)
            .filter(|&pred| pred != action_id)
            .filter_map(|pred| unit_id_for_action.get(&pred).copied())
            .collect();
        for a in &action.after {
            if *a != action_id
                && let Some(uid) = unit_id_for_action.get(a).copied()
            {
                need_set.insert(uid);
            }
        }
        let mut needs: Vec<Ustr> = need_set.into_iter().collect();
        needs.sort();

        let mut ops: Vec<LogicalOp> = Vec::new();

        // A SourceTree input means "this job needs the repo": acquire it in place
        // with a checkout step (rendered as actions/checkout on GitHub), never a
        // download. Non-source inputs use the copy baseline (upload/download) —
        // the legal fallback the placement pass may later upgrade to a mount.
        let needs_source = action.inputs.iter().any(|&id| is_source(id));
        if needs_source {
            ops.push(LogicalOp::CheckoutRepo);
        }
        for &in_id in &action.inputs {
            let art = workflow.artifact(in_id);
            if art.producer.is_none() || art.producer == Some(action_id) || is_source(in_id) {
                continue;
            }
            ops.push(LogicalOp::DownloadArtifact {
                name: art.name,
                path: art.path.clone(),
            });
        }

        // A semantic action defers its concrete realization to backend-agnostic
        // lowering: the inventory's available implementations decide which tool
        // is used. This keeps the workflow expressing intent, not commands.
        let semantic = op_def.and_then(|d| {
            d.implementations.iter().find_map(|i| match i {
                ir::Implementation::Semantic {
                    op,
                    args,
                    using,
                    prefer,
                } => Some((op.clone(), args.clone(), using.clone(), prefer.clone())),
                _ => None,
            })
        });

        let mut dependencies: Vec<Ustr> = Vec::new();
        let mut toolchains: Vec<Ustr> = Vec::new();
        if let Some((op, args, using, prefer)) = semantic {
            let caps: Vec<crate::select::Capability> = actor_for(action, workflow)
                .map(|a| {
                    a.capabilities
                        .iter()
                        .map(crate::select::Capability::new)
                        .collect()
                })
                .unwrap_or_default();
            match lowering_registry.select_using(
                &op,
                &args,
                workflow.inventory.as_ref(),
                &caps,
                using.as_deref(),
                &prefer,
            ) {
                Ok(sel) => {
                    // Surface non-fatal selection warnings (e.g. ambiguous impl).
                    diagnostics.extend(sel.warnings.iter().cloned());
                    // The toolchain the chosen implementation runs on (e.g. cargo
                    // -> rust), provisioned per job from the inventory's version.
                    if let Some(tc) = crate::toolchain::toolchain_for_impl(&sel.implementation) {
                        tc_table
                            .entry(tc.to_string())
                            .or_insert_with(|| inv_version(tc));
                        let tc = Ustr::from(tc);
                        if !toolchains.contains(&tc) {
                            toolchains.push(tc);
                        }
                    }
                    // Record each tool in the shared table once: resolve it to a
                    // package, then lower `package.install` to the install command
                    // (manager pinned for language packages, selected for system).
                    for tool_name in &sel.dependencies {
                        if !dep_table.contains_key(tool_name) {
                            let pref = dep_store.resolve(tool_name);
                            // A language package pins its manager (pip/cargo/...);
                            // a system package uses the actor's system manager,
                            // dnf if the inventory declares it, otherwise apt.
                            let manager = pref
                                .manager
                                .clone()
                                .unwrap_or_else(|| system_manager(workflow.inventory.as_ref()));
                            let mut iargs = crate::lowering::Args::new();
                            iargs.insert(
                                "package".into(),
                                serde_json::Value::String(pref.package.clone()),
                            );
                            if let Ok(install) = lowering_registry.select_using(
                                "package.install",
                                &iargs,
                                workflow.inventory.as_ref(),
                                &[],
                                Some(&manager),
                                &[],
                            ) {
                                let cmd = match install.realization {
                                    crate::lowering::Realization::Shell(c) => c,
                                    crate::lowering::Realization::Native { fallback, .. } => {
                                        fallback
                                    }
                                };
                                dep_table.insert(tool_name.clone(), cmd);
                            }
                        }
                        let tool = Ustr::from(tool_name);
                        if !dependencies.contains(&tool) {
                            dependencies.push(tool);
                        }
                    }
                    dependencies.sort_by(|a, b| a.as_str().cmp(b.as_str()));
                    match sel.realization {
                        crate::lowering::Realization::Shell(script) => {
                            ops.push(LogicalOp::RunShell {
                                label: action.name,
                                script,
                                env: Default::default(),
                            })
                        }
                        crate::lowering::Realization::Native { id, args, fallback } => {
                            ops.push(LogicalOp::Native {
                                id: Ustr::from(&id),
                                args,
                                fallback,
                            })
                        }
                    }
                }
                Err(d) => return Err(vec![d]),
            }
        } else if is_checkout {
            ops.push(LogicalOp::CheckoutRepo);
        } else {
            // Prefer the action's own shell script; fall back to the op
            // definition's first shell-capable implementation.
            let impl_shell = op_def.and_then(|d| d.implementations.iter().find_map(impl_to_shell));
            let effective = action.shell.as_ref().or(impl_shell.as_ref());
            if let Some(sh) = effective {
                ops.push(LogicalOp::RunShell {
                    label: action.name,
                    script: sh.script.clone(),
                    env: sh.env.clone(),
                });
            }
        }

        for &out_id in &action.outputs {
            if is_source(out_id) {
                continue; // SourceTree is reacquired, never uploaded.
            }
            let out = workflow.artifact(out_id);
            ops.push(LogicalOp::UploadArtifact {
                name: out.name,
                path: out.path.clone(),
                lifetime: out.lifetime.clone(),
            });
        }

        let consequences: Vec<ConsequenceInfo> = action
            .consequences
            .iter()
            .map(|&eid| {
                let e = workflow.consequence(eid);
                ConsequenceInfo {
                    name: e.name,
                    kind: e.kind.clone(),
                    requires_approval: e.requires_approval,
                }
            })
            .collect();
        let (consequences_fired, consequences_gated) = partition_consequences(&consequences, event);

        // Insert a RequestApproval op for each approval-gated consequence so
        // backends can lower it to an environment gate / interactive prompt.
        for csq in &consequences_gated {
            if csq.requires_approval {
                ops.push(LogicalOp::RequestApproval {
                    reason: csq.name.to_string(),
                });
            }
        }

        let actor = actor_for(action, workflow);
        let runner = actor
            .and_then(|a| a.labels.first().copied())
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
            consequences,
            consequences_fired,
            consequences_gated,
            secrets_available: event.secrets_available(),
            actor_capabilities,
            timeout: action.timeout.clone(),
            coordination: action.coordination.clone(),
            dependencies,
            toolchains,
        });
    }

    let impl_capabilities = workflow
        .inventory
        .as_ref()
        .map(|inv| {
            inv.implementations
                .iter()
                .filter(|m| !m.deny)
                .map(|m| format!("impl.{}", m.id))
                .collect()
        })
        .unwrap_or_default();

    Ok(Plan {
        workflow_name: workflow.name,
        max_parallel_jobs: workflow.policies.max_parallel_jobs,
        units,
        diagnostics,
        event: event.clone(),
        optimizations: Vec::new(),
        impl_capabilities,
        triggers: workflow.triggers.clone(),
        coordination: workflow.coordination.clone(),
        dependencies: dep_table,
        toolchains: tc_table,
        install_dependencies: workflow.policies.install_dependencies,
    })
}

fn sanitize_id(name: &str) -> String {
    name.to_lowercase().replace([' ', '_'], "-")
}

/// The system package manager for installing system dependencies: dnf when the
/// inventory declares it available, otherwise apt (the common-runner default).
fn system_manager(inv: Option<&ir::Inventory>) -> String {
    let has =
        |id: &str| inv.is_some_and(|i| i.implementations.iter().any(|m| m.id == id && !m.deny));
    if has("dnf") {
        "dnf".into()
    } else {
        "apt".into()
    }
}

pub(crate) fn actor_for<'a>(action: &ActionCall, workflow: &'a Workflow) -> Option<&'a Actor> {
    // An action's resource requirements bias selection toward an actor that
    // satisfies them. A pinned (Specific) actor is honored regardless;
    // validation separately guarantees feasibility.
    let ok = |a: &Actor| action.resources.as_ref().is_none_or(|r| a.satisfies(r));
    for c in &action.actor_constraints {
        match c {
            ActorConstraint::Specific(id) => return Some(workflow.actor(*id)),
            ActorConstraint::Label(label) => {
                let matches = || {
                    workflow
                        .actors()
                        .iter()
                        .filter(|a| a.labels.iter().any(|l| l == label))
                };
                if let Some(a) = matches().find(|a| ok(a)) {
                    return Some(a);
                }
                if let Some(a) = matches().next() {
                    return Some(a);
                }
            }
        }
    }
    workflow
        .actors()
        .iter()
        .find(|a| ok(a))
        .or_else(|| workflow.actors().first())
}

fn topo_sort(workflow: &Workflow) -> Result<Vec<ActionCallId>, Vec<Diagnostic>> {
    let n = workflow.action_calls.len();
    let mut in_degree: Vec<usize> = vec![0; n];
    let mut rdeps: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (idx, action) in workflow.action_calls.iter().enumerate() {
        let mut preds: std::collections::HashSet<usize> = action
            .inputs
            .iter()
            .filter_map(|&art_id| workflow.artifact(art_id).producer)
            .filter(|&p| p.idx() != idx)
            .map(|p| p.idx())
            .collect();
        // Explicit ordering edges (`after`) are dependencies for scheduling too.
        for a in &action.after {
            if a.idx() != idx {
                preds.insert(a.idx());
            }
        }
        in_degree[idx] = preds.len();
        for pred in preds {
            rdeps[pred].push(idx);
        }
    }

    let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut initial: Vec<usize> = queue.drain(..).collect();
    initial.sort();
    queue.extend(initial);

    let mut order: Vec<ActionCallId> = Vec::with_capacity(n);

    while let Some(idx) = queue.pop_front() {
        order.push(ActionCallId(idx as u32));
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
        let pos = |name: &str| {
            plan.units
                .iter()
                .position(|u| u.action_name == name)
                .unwrap()
        };
        assert!(pos("build") < pos("test"));
        // checkout is not a job: source is reacquired per job, not shipped.
        assert!(plan.units.iter().all(|u| u.action_name != "checkout"));
    }

    #[test]
    fn baseline_emits_no_placement_warnings() {
        // The baseline is pure copy/upload-download: always legal, never a
        // "fallback" — placement upgrades (and any fallback warning) belong to
        // the optimization phase, not the planner.
        let plan = plan(&simple_workflow()).unwrap();
        assert!(
            plan.diagnostics
                .iter()
                .all(|d| d.code != DiagCode::FallbackPlacementSelected)
        );
    }

    #[test]
    fn policy_propagated_to_plan() {
        assert_eq!(plan(&simple_workflow()).unwrap().max_parallel_jobs, Some(2));
    }

    #[test]
    fn deployment_effect_preserved_on_action() {
        let mut wf = simple_workflow();
        let eff_id = ConsequenceId(0);
        wf.consequences.push(Consequence {
            name: "deploy".into(),
            kind: ConsequenceKind::Deployment,
            requires_approval: true,
        });
        wf.action_calls[2].consequences.push(eff_id);
        let p = plan(&wf).unwrap();
        let test_unit = p.units.iter().find(|u| u.action_name == "test").unwrap();
        assert!(
            wf.action_call(test_unit.action_id)
                .consequences
                .contains(&eff_id)
        );
    }

    #[test]
    fn source_consumer_checks_out_in_place() {
        let plan = plan(&simple_workflow()).unwrap();
        // No standalone checkout job; the job that needs source checks out itself.
        assert!(plan.units.iter().all(|u| u.action_name != "checkout"));
        let build = plan
            .units
            .iter()
            .find(|u| u.action_name == "build")
            .unwrap();
        assert!(
            build
                .ops
                .iter()
                .any(|op| matches!(op, LogicalOp::CheckoutRepo))
        );
        // SourceTree is never transferred.
        assert!(
            !build
                .ops
                .iter()
                .any(|op| matches!(op, LogicalOp::DownloadArtifact { .. }))
        );
    }

    #[test]
    fn shell_op_emits_run_shell_physical_op() {
        let plan = plan(&simple_workflow()).unwrap();
        let build = plan
            .units
            .iter()
            .find(|u| u.action_name == "build")
            .unwrap();
        assert!(
            build
                .ops
                .iter()
                .any(|op| matches!(op, LogicalOp::RunShell { .. }))
        );
    }

    #[test]
    fn cross_unit_artifact_uses_copy_without_placement() {
        let plan = plan(&simple_workflow()).unwrap();
        // The binary flows build -> test by copy; test downloads it.
        let test = plan.units.iter().find(|u| u.action_name == "test").unwrap();
        assert!(
            test.ops
                .iter()
                .any(|op| matches!(op, LogicalOp::DownloadArtifact { .. }))
        );
        assert!(
            !test
                .ops
                .iter()
                .any(|op| matches!(op, LogicalOp::TransferArtifact { .. }))
        );
    }

    #[test]
    fn baseline_copies_even_with_shared_placement() {
        // A declared shared placement does NOT change the baseline plan; the
        // mount upgrade is the placement optimization pass's job. The planner
        // always produces the correct copy baseline.
        let mut b = WorkflowBuilder::new("mounted");
        let src = b.artifact("source", ArtifactType::SourceTree);
        let bin = b.artifact("binary", ArtifactType::Binary);
        let rep = b.artifact("rep", ArtifactType::TestReport);
        let build = b.shell_action("build", "build", &[src], &[bin], "make");
        let test = b.shell_action("test", "test", &[bin], &[rep], "make test");
        let gpu = b.actor("big", &["self-hosted"], &["mount"]);
        b.constrain_actor(build, gpu);
        b.constrain_actor(test, gpu);
        b.place(
            bin,
            PlacementStrategy::SharedVolume {
                path: "/vol".into(),
            },
        );
        let plan = plan(&b.build()).unwrap();
        let test_unit = plan.units.iter().find(|u| u.action_name == "test").unwrap();
        assert!(
            test_unit
                .ops
                .iter()
                .any(|op| matches!(op, LogicalOp::DownloadArtifact { .. }))
        );
        assert!(
            !test_unit
                .ops
                .iter()
                .any(|op| matches!(op, LogicalOp::TransferArtifact { .. }))
        );
    }

    #[test]
    fn effects_lowered_onto_unit() {
        let mut wf = simple_workflow();
        wf.consequences.push(Consequence {
            name: "deploy".into(),
            kind: ConsequenceKind::Deployment,
            requires_approval: true,
        });
        wf.action_calls[2].consequences.push(ConsequenceId(0));
        let plan = plan(&wf).unwrap();
        let test_unit = plan.units.iter().find(|u| u.action_name == "test").unwrap();
        assert_eq!(test_unit.consequences.len(), 1);
        assert_eq!(test_unit.consequences[0].name, "deploy");
        assert!(test_unit.consequences[0].requires_approval);
    }

    fn deploy_workflow(approval: bool) -> Workflow {
        let mut b = WorkflowBuilder::new("w");
        let bin = b.artifact("bin", ArtifactType::Binary);
        let a = b.shell_action("deploy", "deploy", &[], &[bin], "x");
        b.add_secrets(a, &["TOKEN"]);
        let e = b.consequence("ship", ConsequenceKind::Deployment, approval);
        b.add_consequence_to(a, e);
        b.actor("l", &["ubuntu-latest"], &[]);
        b.build()
    }

    #[test]
    fn effect_fires_on_push_gated_on_pr() {
        let wf = deploy_workflow(false);
        let push = plan_for(
            &wf,
            &EventContext::Push {
                branch: "main".into(),
            },
        )
        .unwrap();
        assert!(
            push.units[0]
                .consequences_fired
                .iter()
                .any(|e| e.name == "ship")
        );
        assert!(push.units[0].consequences_gated.is_empty());

        let pr = plan_for(&wf, &EventContext::PullRequest { fork: false }).unwrap();
        assert!(
            pr.units[0]
                .consequences_gated
                .iter()
                .any(|e| e.name == "ship")
        );
        assert!(pr.units[0].consequences_fired.is_empty());
    }

    #[test]
    fn approval_effect_gated_even_on_push() {
        let push = plan_for(&deploy_workflow(true), &EventContext::default()).unwrap();
        assert!(
            push.units[0]
                .consequences_gated
                .iter()
                .any(|e| e.name == "ship")
        );
        assert!(push.units[0].consequences_fired.is_empty());
    }

    #[test]
    fn secrets_available_depends_on_event() {
        let wf = deploy_workflow(false);
        assert!(plan_for(&wf, &EventContext::default()).unwrap().units[0].secrets_available);
        assert!(
            !plan_for(&wf, &EventContext::PullRequest { fork: true })
                .unwrap()
                .units[0]
                .secrets_available
        );
    }

    #[test]
    fn actor_capabilities_recorded_on_unit() {
        let mut b = WorkflowBuilder::new("w");
        let bin = b.artifact("bin", ArtifactType::Binary);
        let a = b.shell_action("build", "build", &[], &[bin], "make");
        let gpu = b.actor("gpu-box", &["self-hosted"], &["gpu", "cuda"]);
        b.constrain_actor(a, gpu);
        let plan = plan(&b.build()).unwrap();
        assert_eq!(
            plan.units[0].actor_capabilities,
            vec!["gpu".to_string(), "cuda".to_string()]
        );
    }

    #[test]
    fn plan_records_its_event() {
        let plan = plan_for(
            &deploy_workflow(false),
            &EventContext::Tag { name: "v1".into() },
        )
        .unwrap();
        assert!(matches!(plan.event, EventContext::Tag { .. }));
    }

    #[test]
    fn artifact_path_lowered_into_ops() {
        let mut b = WorkflowBuilder::new("w");
        let bin = b.artifact_at("bin", ArtifactType::Binary, "target/app");
        b.shell_action("build", "build", &[], &[bin], "make");
        b.actor("l", &["ubuntu-latest"], &[]);
        let plan = plan(&b.build()).unwrap();
        assert!(plan.units[0].ops.iter().any(
            |op| matches!(op, LogicalOp::UploadArtifact { path: Some(p), .. } if p == "target/app")
        ));
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

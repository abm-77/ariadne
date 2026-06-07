//! Backend-agnostic workflow testing. Plan-level assertions are evaluated with
//! no execution (against any `Backend`); execution-level assertions run the
//! workflow through an `Executor` (the local Podman backend implements one).

use crate::backends::github::GithubActionsBackend;
use crate::backends::{Backend, Selector};
use crate::ir::Workflow;
use crate::planner::{ConsequenceInfo, ConsequenceKind, LogicalOp, Plan, plan_for};
use serde::{Deserialize, Serialize};

pub use crate::planner::EventContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferKind {
    Copy,
    Mount,
}

#[derive(Debug, Clone)]
pub struct ArtifactTransfer {
    pub name: String,
    pub path: Option<String>,
    pub kind: TransferKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitStatus {
    Passed,
    Failed(i32),
    Skipped,
}

#[derive(Debug, Clone)]
pub struct UnitOutcome {
    pub id: String,
    pub action_name: String,
    pub status: UnitStatus,
    pub stdout: String,
    pub stderr: String,
    pub artifacts: Vec<String>,
    pub secrets_spoofed: Vec<String>,
    pub secrets_withheld: Vec<String>,
    pub consequences_fired: Vec<ConsequenceInfo>,
    pub consequences_gated: Vec<ConsequenceInfo>,
    pub transfers: Vec<ArtifactTransfer>,
}

impl UnitOutcome {
    pub fn passed(&self) -> bool {
        self.status == UnitStatus::Passed
    }
}

/// Result of executing a workflow. Effects never fire and secrets are spoofed;
/// everything here is meant to be asserted against.
#[derive(Debug, Clone)]
pub struct TestRun {
    pub workflow_name: String,
    pub units: Vec<UnitOutcome>,
}

impl TestRun {
    pub fn passed(&self) -> bool {
        self.units.iter().all(|u| u.status == UnitStatus::Passed)
    }
    pub fn unit(&self, name: &str) -> Option<&UnitOutcome> {
        self.units.iter().find(|u| u.action_name == name)
    }
    pub fn artifact_produced(&self, name: &str) -> bool {
        self.units
            .iter()
            .any(|u| u.artifacts.iter().any(|a| a == name))
    }
    pub fn transfer_of(&self, artifact: &str) -> Option<TransferKind> {
        self.units
            .iter()
            .flat_map(|u| &u.transfers)
            .find(|t| t.name == artifact)
            .map(|t| t.kind)
    }
    pub fn effect_fired(&self, name: &str) -> bool {
        self.units
            .iter()
            .any(|u| u.consequences_fired.iter().any(|e| e.name == name))
    }
    pub fn effect_gated(&self, name: &str) -> bool {
        self.units
            .iter()
            .any(|u| u.consequences_gated.iter().any(|e| e.name == name))
    }
}

/// Prefix marking a spoofed secret value, so scripts/asserts can tell a test
/// run never received a real credential.
pub const SPOOF_PREFIX: &str = "spoofed";

pub fn spoof_value(name: &str) -> String {
    format!("{SPOOF_PREFIX}-{name}")
}

/// Artifact transfers a unit performs, read from its lowered ops.
pub fn transfers_of(unit: &crate::planner::ExecutionUnit) -> Vec<ArtifactTransfer> {
    use crate::planner::AccessMode;
    unit.ops
        .iter()
        .filter_map(|op| match op {
            LogicalOp::DownloadArtifact { name, path } => Some(ArtifactTransfer {
                name: name.to_string(),
                path: path.clone(),
                kind: TransferKind::Copy,
            }),
            LogicalOp::TransferArtifact {
                name,
                path,
                access: AccessMode::MountReadOnly | AccessMode::MountReadWrite,
            } => Some(ArtifactTransfer {
                name: name.to_string(),
                path: path.clone(),
                kind: TransferKind::Mount,
            }),
            LogicalOp::TransferArtifact {
                name,
                path,
                access: AccessMode::Copy,
            } => Some(ArtifactTransfer {
                name: name.to_string(),
                path: path.clone(),
                kind: TransferKind::Copy,
            }),
            _ => None,
        })
        .collect()
}

/// Artifact names a unit produces (uploads).
pub fn produced_artifacts(unit: &crate::planner::ExecutionUnit) -> Vec<String> {
    unit.ops
        .iter()
        .filter_map(|op| match op {
            LogicalOp::UploadArtifact { name, .. } => Some(name.to_string()),
            _ => None,
        })
        .collect()
}

/// A backend that can actually run a plan and report results. Execution is a
/// capability some backends have; assertion checking
/// does not require it.
pub trait Executor {
    fn execute(&self, plan: &Plan) -> Result<TestRun, String>;
}

/// Internal representation of a workflow-test assertion. A frontend generates
/// these; the checker evaluates them against a plan (plan-level) or a `TestRun`
/// (execution-level).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "assert")]
pub enum Assertion {
    // Plan-level — evaluated against the plan + event with NO execution.
    ArtifactProduced {
        artifact: String,
    },
    HasConsequence {
        effect: String,
    },
    ConsequenceRequiresApproval {
        effect: String,
    },
    ConsequenceFired {
        effect: String,
    },
    ConsequenceGated {
        effect: String,
    },
    SecretSpoofed {
        secret: String,
    },
    SecretWithheld {
        secret: String,
    },
    TransferUsed {
        artifact: String,
        kind: TransferKind,
    },
    ArtifactPath {
        artifact: String,
        path: String,
    },
    MaxParallelJobs {
        max: usize,
    },
    MaxJobsWithCapability {
        capability: String,
        max: usize,
    },
    MaxConcurrentDeployments {
        max: usize,
    },
    SelectedInstruction {
        op: String,
        instruction: String,
    },
    HasWarning {
        contains: String,
    },
    // Execution-level — require a real run via an `Executor`.
    RunPassed,
    RunFailed,
    UnitPassed {
        unit: String,
    },
    UnitFailed {
        unit: String,
    },
    UnitSkipped {
        unit: String,
    },
    StdoutContains {
        unit: String,
        text: String,
    },
}

impl Assertion {
    /// Whether this assertion needs the workflow to actually run. Everything
    /// else is decidable from the plan alone.
    pub fn requires_execution(&self) -> bool {
        matches!(
            self,
            Assertion::RunPassed
                | Assertion::RunFailed
                | Assertion::UnitPassed { .. }
                | Assertion::UnitFailed { .. }
                | Assertion::UnitSkipped { .. }
                | Assertion::StdoutContains { .. }
        )
    }
}

#[derive(Debug, Clone)]
pub struct AssertionResult {
    pub assertion: Assertion,
    pub passed: bool,
    pub detail: String,
}

/// A self-contained test case: an embedded workflow, event, and assertions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub workflow: Workflow,
    #[serde(default)]
    pub event: EventContext,
    pub assertions: Vec<Assertion>,
}

/// One named case within a suite. The workflow comes from the `.tir` file the
/// suite accompanies; the case supplies event, optional target backend (for
/// instruction-selection assertions), and assertions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    pub name: String,
    #[serde(default)]
    pub event: EventContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    pub assertions: Vec<Assertion>,
}

/// The cases to run against a workflow `.tir` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSuite {
    pub cases: Vec<TestCase>,
}

impl TestSuite {
    pub fn load(path: &std::path::Path) -> Result<TestSuite, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("parse '{}': {e}", path.display()))
    }
}

#[derive(Debug, Clone)]
pub struct CaseReport {
    pub name: String,
    /// Present only if execution-level assertions forced a real run.
    pub run: Option<TestRun>,
    pub results: Vec<AssertionResult>,
}

impl CaseReport {
    pub fn passed(&self) -> bool {
        self.results.iter().all(|r| r.passed)
    }
    pub fn failures(&self) -> impl Iterator<Item = &AssertionResult> {
        self.results.iter().filter(|r| !r.passed)
    }
}

/// Plan and check a self-contained fixture. Plan-level assertions need no
/// execution; the workflow runs only if execution-level assertions are present.
pub fn run_fixture<E: Backend + Executor>(
    name: &str,
    fixture: &Fixture,
    backend: &E,
) -> Result<CaseReport, String> {
    let case = TestCase {
        name: name.to_string(),
        event: fixture.event.clone(),
        backend: None,
        assertions: fixture.assertions.clone(),
    };
    run_case(&fixture.workflow, &case, backend)
}

/// Run one suite case against a workflow. Plan-level assertions are evaluated
/// with no execution (using the case's named backend for instruction
/// selection); execution-level assertions run via the executor.
pub fn run_case<E: Backend + Executor>(
    workflow: &Workflow,
    case: &TestCase,
    backend: &E,
) -> Result<CaseReport, String> {
    let baseline =
        plan_for(workflow, &case.event).map_err(|diags| format!("planning failed: {diags:?}"))?;

    let inv_caps =
        crate::backends::derive_capability_profile_from_inventory(workflow.inventory.as_ref());
    // Assertions are evaluated against the *optimized* plan (default level),
    // using the capabilities of whichever backend the case selects.
    let mut results = match case.backend.as_deref() {
        Some("github") => {
            let gh = GithubActionsBackend::default();
            check_plan(
                &optimize_for(workflow, baseline.clone(), inv_caps),
                &gh,
                &case.assertions,
            )
        }
        Some(other) if other != "local" => {
            return Err(format!("unknown backend '{other}' (expected github|local)"));
        }
        _ => check_plan(
            &optimize_for(workflow, baseline.clone(), inv_caps),
            backend,
            &case.assertions,
        ),
    };

    let mut run = None;
    if case.assertions.iter().any(Assertion::requires_execution) {
        let plan = optimize_for(workflow, baseline, inv_caps);
        let r = backend.execute(&plan)?;
        for a in case.assertions.iter().filter(|a| a.requires_execution()) {
            results.push(evaluate_exec(a, &r));
        }
        run = Some(r);
    }

    Ok(CaseReport {
        name: case.name.clone(),
        run,
        results,
    })
}

/// Optimize a baseline plan at the default level for a backend's capabilities,
/// reading the objective ordering from the workflow's policies. Exposed so
/// callers asserting against plan shape see the same optimized plan `run_case`
/// does.
pub fn optimize_for(
    workflow: &Workflow,
    plan: Plan,
    caps: crate::backends::BackendCapabilities,
) -> Plan {
    let profile = crate::profile::Profile::default();
    let analysis = crate::analysis::Analysis::of(workflow);
    let ctx = crate::optimize::OptimizeCtx {
        workflow,
        profile: &profile,
        backend_caps: caps,
        policy: &workflow.policies,
        analysis: &analysis,
        objectives: workflow.policies.objectives.clone(),
        level: crate::optimize::OptLevel::default(),
    };
    crate::optimize::optimize(plan, &ctx)
}

/// Evaluate the plan-level assertions against an (event-aware) plan, with no
/// execution. Execution-level assertions are skipped here.
pub fn check_plan<B: Backend>(
    plan: &Plan,
    backend: &B,
    assertions: &[Assertion],
) -> Vec<AssertionResult> {
    let view = PlanView::build(plan);
    assertions
        .iter()
        .filter(|a| !a.requires_execution())
        .map(|a| evaluate_plan(a, plan, &view, backend))
        .collect()
}

/// Aggregated plan facts, read from the event-baked plan with no execution.
struct PlanView {
    artifacts: Vec<String>,
    fired: Vec<ConsequenceInfo>,
    gated: Vec<ConsequenceInfo>,
    transfers: Vec<ArtifactTransfer>,
    secrets: Vec<String>,
    secrets_available: bool,
}

impl PlanView {
    fn build(plan: &Plan) -> Self {
        Self {
            artifacts: plan.units.iter().flat_map(produced_artifacts).collect(),
            fired: plan
                .units
                .iter()
                .flat_map(|u| u.consequences_fired.clone())
                .collect(),
            gated: plan
                .units
                .iter()
                .flat_map(|u| u.consequences_gated.clone())
                .collect(),
            transfers: plan.units.iter().flat_map(transfers_of).collect(),
            secrets: plan
                .units
                .iter()
                .flat_map(|u| u.secrets.iter().map(|s| s.to_string()))
                .collect(),
            secrets_available: plan.event.secrets_available(),
        }
    }
}

fn result(assertion: &Assertion, passed: bool, detail: String) -> AssertionResult {
    AssertionResult {
        assertion: assertion.clone(),
        passed,
        detail,
    }
}

fn evaluate_plan<B: Backend>(
    a: &Assertion,
    plan: &Plan,
    view: &PlanView,
    backend: &B,
) -> AssertionResult {
    let (passed, detail) = match a {
        Assertion::ArtifactProduced { artifact } => (
            view.artifacts.iter().any(|x| x == artifact),
            format!("artifact '{artifact}' not produced"),
        ),
        Assertion::HasConsequence { effect } => (
            view.fired
                .iter()
                .chain(&view.gated)
                .any(|e| e.name == effect),
            format!("no effect named '{effect}'"),
        ),
        Assertion::ConsequenceRequiresApproval { effect } => (
            view.fired
                .iter()
                .chain(&view.gated)
                .any(|e| e.name == effect && e.requires_approval),
            format!("effect '{effect}' does not require approval"),
        ),
        Assertion::ConsequenceFired { effect } => (
            view.fired.iter().any(|e| e.name == effect),
            format!("effect '{effect}' did not fire"),
        ),
        Assertion::ConsequenceGated { effect } => (
            view.gated.iter().any(|e| e.name == effect),
            format!("effect '{effect}' was not gated"),
        ),
        Assertion::SecretSpoofed { secret } => (
            view.secrets_available && view.secrets.iter().any(|s| s == secret),
            format!("secret '{secret}' was not available/spoofed"),
        ),
        Assertion::SecretWithheld { secret } => (
            !view.secrets_available && view.secrets.iter().any(|s| s == secret),
            format!("secret '{secret}' was not withheld"),
        ),
        Assertion::TransferUsed { artifact, kind } => {
            let got = view
                .transfers
                .iter()
                .find(|t| &t.name == artifact)
                .map(|t| t.kind);
            (
                got == Some(*kind),
                format!("artifact '{artifact}' did not use {kind:?} transfer (got {got:?})"),
            )
        }
        Assertion::ArtifactPath { artifact, path } => {
            let got = plan
                .units
                .iter()
                .flat_map(|u| &u.ops)
                .find_map(|op| match op {
                    LogicalOp::UploadArtifact {
                        name,
                        path: Some(p),
                        ..
                    } if name == artifact => Some(p.clone()),
                    LogicalOp::DownloadArtifact {
                        name,
                        path: Some(p),
                    } if name == artifact => Some(p.clone()),
                    LogicalOp::TransferArtifact {
                        name,
                        path: Some(p),
                        ..
                    } if name == artifact => Some(p.clone()),
                    _ => None,
                });
            (
                got.as_deref() == Some(path.as_str()),
                format!("artifact '{artifact}' path is {got:?}, expected '{path}'"),
            )
        }
        Assertion::MaxParallelJobs { max } => (
            plan.max_parallel_jobs.is_none_or(|n| n <= *max),
            format!(
                "max_parallel_jobs {:?} exceeds {max}",
                plan.max_parallel_jobs
            ),
        ),
        Assertion::MaxJobsWithCapability { capability, max } => {
            let n = plan
                .units
                .iter()
                .filter(|u| u.actor_capabilities.iter().any(|c| c == capability))
                .count();
            (
                n <= *max,
                format!("{n} jobs need capability '{capability}', exceeds {max}"),
            )
        }
        Assertion::MaxConcurrentDeployments { max } => {
            let n = plan
                .units
                .iter()
                .filter(|u| {
                    u.consequences_fired
                        .iter()
                        .any(|e| e.kind == ConsequenceKind::Deployment)
                })
                .count();
            (
                n <= *max,
                format!("{n} concurrent deployments, exceeds {max}"),
            )
        }
        Assertion::SelectedInstruction { op, instruction } => {
            let caps = backend.capabilities();
            let selector = Selector::for_backend(backend);
            let found = plan
                .units
                .iter()
                .flat_map(|u| &u.ops)
                .find(|o| o.action() == *op)
                .and_then(|o| selector.select(o, &caps, &[]))
                .map(|sel| sel.instruction.id.0.to_string());
            (
                found.as_deref() == Some(instruction.as_str()),
                format!("op '{op}' selected {found:?}, expected '{instruction}'"),
            )
        }
        Assertion::HasWarning { contains } => (
            plan.diagnostics
                .iter()
                .any(|d| d.to_string().contains(contains)),
            format!("no warning containing '{contains}'"),
        ),
        _ => (true, String::new()),
    };
    result(a, passed, detail)
}

fn evaluate_exec(a: &Assertion, run: &TestRun) -> AssertionResult {
    let (passed, detail) = match a {
        Assertion::RunPassed => (run.passed(), "run did not pass".into()),
        Assertion::RunFailed => (!run.passed(), "run unexpectedly passed".into()),
        Assertion::UnitPassed { unit } => unit_status_is(run, unit, |s| *s == UnitStatus::Passed),
        Assertion::UnitFailed { unit } => {
            unit_status_is(run, unit, |s| matches!(s, UnitStatus::Failed(_)))
        }
        Assertion::UnitSkipped { unit } => unit_status_is(run, unit, |s| *s == UnitStatus::Skipped),
        Assertion::StdoutContains { unit, text } => match run.unit(unit) {
            Some(u) => (
                u.stdout.contains(text),
                format!("unit '{unit}' stdout missing '{text}'"),
            ),
            None => (false, format!("no unit named '{unit}'")),
        },
        _ => (true, String::new()),
    };
    result(a, passed, detail)
}

fn unit_status_is(run: &TestRun, name: &str, pred: impl Fn(&UnitStatus) -> bool) -> (bool, String) {
    match run.unit(name) {
        Some(u) => (
            pred(&u.status),
            format!("unit '{name}' status was {:?}", u.status),
        ),
        None => (false, format!("no unit named '{name}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::local::LocalBackend;
    use crate::ir::{ArtifactType, ConsequenceKind, WorkflowBuilder};

    fn rich_wf() -> Workflow {
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::Binary);
        let bin = b.artifact_at("bin", ArtifactType::Binary, "out/app");
        let co = b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        let build = b.shell_action("build", "build", &[src], &[bin], "make");
        let deploy = b.shell_action("deploy", "deploy", &[bin], &[], "./d");
        b.add_secrets(build, &["TOKEN"]);
        let rel = b.consequence("release", ConsequenceKind::PublishRelease, true);
        b.add_consequence_to(build, rel);
        let ship = b.consequence("ship", ConsequenceKind::Deployment, false);
        b.add_consequence_to(deploy, ship);
        let runner = b.actor("r", &["ubuntu-latest"], &[]);
        let gpu = b.actor("g", &["self-hosted"], &["gpu"]);
        b.constrain_actor(co, runner);
        b.constrain_actor(build, gpu);
        b.constrain_actor(deploy, runner);
        b.max_parallel_jobs(4);
        b.build()
    }

    fn eval(plan: &Plan, a: Assertion) -> bool {
        let backend = LocalBackend::podman();
        check_plan(plan, &backend, std::slice::from_ref(&a))
            .into_iter()
            .next()
            .map(|r| r.passed)
            .unwrap_or(false)
    }

    fn push_plan() -> Plan {
        plan_for(&rich_wf(), &EventContext::default()).unwrap()
    }

    #[test]
    fn check_plan_skips_execution_level() {
        let backend = LocalBackend::podman();
        let r = check_plan(&push_plan(), &backend, &[Assertion::RunPassed]);
        assert!(r.is_empty());
    }

    #[test]
    fn effect_assertions() {
        let p = push_plan();
        assert!(eval(
            &p,
            Assertion::HasConsequence {
                effect: "release".into()
            }
        ));
        assert!(!eval(
            &p,
            Assertion::HasConsequence {
                effect: "nope".into()
            }
        ));
        assert!(eval(
            &p,
            Assertion::ConsequenceRequiresApproval {
                effect: "release".into()
            }
        ));
        assert!(eval(
            &p,
            Assertion::ConsequenceGated {
                effect: "release".into()
            }
        ));
        assert!(eval(
            &p,
            Assertion::ConsequenceFired {
                effect: "ship".into()
            }
        ));
    }

    #[test]
    fn secret_assertions_by_event() {
        assert!(eval(
            &push_plan(),
            Assertion::SecretSpoofed {
                secret: "TOKEN".into()
            }
        ));
        let fork = plan_for(&rich_wf(), &EventContext::PullRequest { fork: true }).unwrap();
        assert!(eval(
            &fork,
            Assertion::SecretWithheld {
                secret: "TOKEN".into()
            }
        ));
        assert!(!eval(
            &fork,
            Assertion::SecretSpoofed {
                secret: "TOKEN".into()
            }
        ));
    }

    #[test]
    fn transfer_path_and_policy_assertions() {
        let p = push_plan();
        assert!(eval(
            &p,
            Assertion::TransferUsed {
                artifact: "src".into(),
                kind: TransferKind::Copy
            }
        ));
        assert!(eval(
            &p,
            Assertion::ArtifactPath {
                artifact: "bin".into(),
                path: "out/app".into()
            }
        ));
        assert!(!eval(
            &p,
            Assertion::ArtifactPath {
                artifact: "bin".into(),
                path: "x".into()
            }
        ));
        assert!(eval(&p, Assertion::MaxParallelJobs { max: 10 }));
        assert!(!eval(&p, Assertion::MaxParallelJobs { max: 1 }));
        assert!(eval(
            &p,
            Assertion::MaxJobsWithCapability {
                capability: "gpu".into(),
                max: 2
            }
        ));
        assert!(!eval(
            &p,
            Assertion::MaxJobsWithCapability {
                capability: "gpu".into(),
                max: 0
            }
        ));
        assert!(eval(&p, Assertion::MaxConcurrentDeployments { max: 1 }));
        assert!(!eval(&p, Assertion::MaxConcurrentDeployments { max: 0 }));
    }

    #[test]
    fn selection_assertion() {
        assert!(eval(
            &push_plan(),
            Assertion::SelectedInstruction {
                op: "scm.checkout".into(),
                instruction: "local.semantic.fallback".into()
            }
        ));
    }

    #[test]
    fn placement_fallback_warns_on_no_mount_backend() {
        // Actor has no mount capability so inv_caps has no MOUNTS; the
        // SharedVolume placement must fall back to copy and surface a warning.
        use crate::ir::{ArtifactType, PlacementStrategy, WorkflowBuilder};
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::Binary);
        let bin = b.artifact("bin", ArtifactType::Binary);
        let co = b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        let build = b.shell_action("build", "build", &[src], &[bin], "make");
        let actor = b.actor("m", &["self-hosted"], &[]);
        b.constrain_actor(co, actor);
        b.constrain_actor(build, actor);
        b.place(
            src,
            PlacementStrategy::SharedVolume {
                path: "/vol".into(),
            },
        );
        let wf = b.build();
        let case = TestCase {
            name: "fb".into(),
            event: EventContext::default(),
            backend: Some("github".into()),
            assertions: vec![Assertion::HasWarning {
                contains: "fallback".into(),
            }],
        };
        let report = run_case(&wf, &case, &LocalBackend::podman()).unwrap();
        assert!(
            report.passed(),
            "{:?}",
            report.failures().collect::<Vec<_>>()
        );
    }

    #[test]
    fn run_case_github_backend_for_selection() {
        let backend = LocalBackend::podman();
        let case = TestCase {
            name: "gh".into(),
            event: EventContext::default(),
            backend: Some("github".into()),
            assertions: vec![Assertion::SelectedInstruction {
                op: "scm.checkout".into(),
                instruction: "github.checkout.native".into(),
            }],
        };
        let report = run_case(&rich_wf(), &case, &backend).unwrap();
        assert!(
            report.passed(),
            "{:?}",
            report.failures().collect::<Vec<_>>()
        );
        assert!(report.run.is_none());
    }

    #[test]
    fn run_case_unknown_backend_errors() {
        let backend = LocalBackend::podman();
        let case = TestCase {
            name: "x".into(),
            event: EventContext::default(),
            backend: Some("gitlab".into()),
            assertions: vec![],
        };
        assert!(run_case(&rich_wf(), &case, &backend).is_err());
    }

    #[test]
    fn requires_execution_classification() {
        assert!(Assertion::RunPassed.requires_execution());
        assert!(!Assertion::ConsequenceGated { effect: "x".into() }.requires_execution());
    }

    #[test]
    fn test_suite_parses() {
        let json = r#"{ "cases": [
            { "name": "c1", "event": {"pull_request":{"fork":true}},
              "assertions": [{"assert":"secret_withheld","secret":"T"}] },
            { "name": "c2", "backend": "github", "assertions": [{"assert":"run_passed"}] }
        ] }"#;
        let suite: TestSuite = serde_json::from_str(json).unwrap();
        assert_eq!(suite.cases.len(), 2);
        assert_eq!(suite.cases[1].backend.as_deref(), Some("github"));
    }

    #[test]
    fn assertion_roundtrips_through_json() {
        let a = Assertion::TransferUsed {
            artifact: "x".into(),
            kind: TransferKind::Mount,
        };
        let s = serde_json::to_string(&a).unwrap();
        assert!(s.contains("\"assert\":\"transfer_used\""));
        assert!(matches!(
            serde_json::from_str::<Assertion>(&s).unwrap(),
            Assertion::TransferUsed {
                kind: TransferKind::Mount,
                ..
            }
        ));
    }
}

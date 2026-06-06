//! Local execution: the `Executor` impl that actually runs a plan in Podman.
//! Lowering and rendering go through the backend's `lower`; this module only
//! orchestrates containers, secret spoofing, and the mock artifact store.

use crate::backends::local::runtime::{ContainerRuntime, ContainerSpec, Mount};
use crate::backends::local::LocalBackend;
use crate::backends::Backend;
use crate::planner::{ExecutionUnit, Plan};
use crate::testing::{
    produced_artifacts, spoof_value, transfers_of, ArtifactTransfer, Executor, TestRun,
    UnitOutcome, UnitStatus,
};

impl<R: ContainerRuntime> Executor for LocalBackend<R> {
    /// Run each unit in a container, in topological order. The plan is already
    /// event-aware; secrets are spoofed/withheld accordingly, effects are
    /// recorded (never executed), and the first failing unit aborts the rest.
    /// Shell lines come from the backend's `lower` (a `BashScript`), so there is
    /// no bespoke lowering here.
    fn execute(&self, plan: &Plan) -> Result<TestRun, String> {
        self.runtime.pull(&self.opts.default_image).map_err(|e| e.to_string())?;

        let cwd = std::env::current_dir().map_err(|e| format!("cannot resolve cwd: {e}"))?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos()).unwrap_or(0);
        let store = std::env::temp_dir().join(format!("loom-store-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&store).map_err(|e| format!("cannot create artifact store: {e}"))?;
        let store_host = store.to_string_lossy().into_owned();

        let script = self.lower(plan);
        let mut outcomes: Vec<UnitOutcome> = Vec::new();
        let mut aborted = false;

        for (unit, bash_unit) in plan.units.iter().zip(&script.units) {
            let fired = unit.consequences_fired.clone();
            let gated = unit.consequences_gated.clone();
            let transfers = transfers_of(unit);
            let artifacts = produced_artifacts(unit);

            if aborted {
                outcomes.push(skipped(unit, fired, gated, transfers, artifacts));
                continue;
            }

            let (spoofed, withheld, mut env) = resolve_secrets(unit);
            let status;
            let mut stdout = String::new();
            let mut stderr = String::new();

            if bash_unit.lines.is_empty() {
                status = UnitStatus::Passed;
            } else {
                env.push(("LOOM_ARTIFACT_STORE".into(), "/loom-artifacts".into()));
                let script_text = format!("set -euo pipefail\n{}", bash_unit.lines.join("\n"));
                let spec = ContainerSpec {
                    image: self.opts.default_image.clone(),
                    command: vec!["bash".into(), "-c".into(), script_text],
                    env,
                    mounts: vec![
                        Mount { host: cwd.to_string_lossy().into_owned(), container: self.opts.workdir.clone(), read_only: false },
                        Mount { host: store_host.clone(), container: "/loom-artifacts".into(), read_only: false },
                    ],
                    workdir: Some(self.opts.workdir.clone()),
                    remove_after: true,
                };
                let result = self.runtime.run(&spec).map_err(|e| e.to_string())?;
                stdout = String::from_utf8_lossy(&result.stdout).into_owned();
                stderr = String::from_utf8_lossy(&result.stderr).into_owned();
                status = if result.exit_code == 0 {
                    UnitStatus::Passed
                } else {
                    aborted = true;
                    UnitStatus::Failed(result.exit_code)
                };
            }

            outcomes.push(UnitOutcome {
                id: unit.id.to_string(),
                action_name: unit.action_name.to_string(),
                status,
                stdout,
                stderr,
                artifacts,
                secrets_spoofed: spoofed,
                secrets_withheld: withheld,
                consequences_fired: fired,
                consequences_gated: gated,
                transfers,
            });
        }

        Ok(TestRun { workflow_name: plan.workflow_name.to_string(), units: outcomes })
    }
}

fn resolve_secrets(unit: &ExecutionUnit) -> (Vec<String>, Vec<String>, Vec<(String, String)>) {
    if unit.secrets_available {
        let env = unit.secrets.iter().map(|n| (n.to_string(), spoof_value(n))).collect();
        (unit.secrets.iter().map(|n| n.to_string()).collect(), Vec::new(), env)
    } else {
        (Vec::new(), unit.secrets.iter().map(|n| n.to_string()).collect(), Vec::new())
    }
}

fn skipped(
    unit: &ExecutionUnit,
    fired: Vec<crate::planner::ConsequenceInfo>,
    gated: Vec<crate::planner::ConsequenceInfo>,
    transfers: Vec<ArtifactTransfer>,
    artifacts: Vec<String>,
) -> UnitOutcome {
    UnitOutcome {
        id: unit.id.to_string(),
        action_name: unit.action_name.to_string(),
        status: UnitStatus::Skipped,
        stdout: String::new(),
        stderr: String::new(),
        artifacts,
        secrets_spoofed: Vec::new(),
        secrets_withheld: unit.secrets.iter().map(|n| n.to_string()).collect(),
        consequences_fired: fired,
        consequences_gated: gated,
        transfers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::local::runtime::{RunResult, RuntimeError};
    use crate::ir::{ArtifactType, ConsequenceKind, Workflow, WorkflowBuilder};
    use crate::planner::{plan, plan_for, EventContext};
    use crate::testing::TransferKind;
    use std::sync::Mutex;

    struct MockRuntime {
        specs: Mutex<Vec<ContainerSpec>>,
        exit_codes: Mutex<Vec<i32>>,
    }
    impl MockRuntime {
        fn with_exit_codes(codes: Vec<i32>) -> Self {
            Self { specs: Mutex::new(Vec::new()), exit_codes: Mutex::new(codes) }
        }
    }
    impl ContainerRuntime for MockRuntime {
        fn name(&self) -> &str { "mock" }
        fn pull(&self, _: &str) -> Result<(), RuntimeError> { Ok(()) }
        fn run(&self, spec: &ContainerSpec) -> Result<RunResult, RuntimeError> {
            let code = { let mut c = self.exit_codes.lock().unwrap(); if c.is_empty() { 0 } else { c.remove(0) } };
            self.specs.lock().unwrap().push(ContainerSpec {
                image: spec.image.clone(), command: spec.command.clone(), env: spec.env.clone(),
                mounts: spec.mounts.iter().map(|m| Mount { host: m.host.clone(), container: m.container.clone(), read_only: m.read_only }).collect(),
                workdir: spec.workdir.clone(), remove_after: spec.remove_after,
            });
            Ok(RunResult { exit_code: code, stdout: b"ok".to_vec(), stderr: Vec::new() })
        }
    }

    fn wf_secret_effect() -> Workflow {
        let mut b = WorkflowBuilder::new("test");
        let src = b.artifact("source", ArtifactType::SourceTree);
        let bin = b.artifact("binary", ArtifactType::Binary);
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        let build = b.shell_action("build", "build", &[src], &[bin], "cargo build --release");
        b.add_secrets(build, &["DEPLOY_TOKEN"]);
        let deploy = b.consequence("deploy", ConsequenceKind::Deployment, false);
        b.add_consequence_to(build, deploy);
        b.actor("local", &["ubuntu-latest"], &[]);
        b.build()
    }

    fn backend(codes: Vec<i32>) -> LocalBackend<MockRuntime> {
        LocalBackend::new(MockRuntime::with_exit_codes(codes))
    }

    #[test]
    fn all_units_pass_when_exit_zero() {
        let run = backend(vec![0, 0]).execute(&plan(&wf_secret_effect()).unwrap()).unwrap();
        assert!(run.unit("checkout").unwrap().passed());
        assert!(run.unit("build").unwrap().passed());
    }

    #[test]
    fn produced_artifacts_recorded() {
        let run = backend(vec![0, 0]).execute(&plan(&wf_secret_effect()).unwrap()).unwrap();
        assert!(run.artifact_produced("binary"));
    }

    #[test]
    fn defaults_to_ubuntu_and_workspace() {
        let backend = backend(vec![0, 0]);
        backend.execute(&plan(&wf_secret_effect()).unwrap()).unwrap();
        let specs = backend.runtime.specs.lock().unwrap();
        let s = specs.first().unwrap();
        assert_eq!(s.image, "ubuntu:24.04");
        assert_eq!(s.workdir.as_deref(), Some("/workspace"));
    }

    #[test]
    fn with_image_and_workdir_propagate_to_container_spec() {
        let backend = LocalBackend::new(MockRuntime::with_exit_codes(vec![0, 0]))
            .with_image("rust:1")
            .with_workdir("/build");
        let run = backend.execute(&plan(&wf_secret_effect()).unwrap()).unwrap();
        assert!(run.unit("checkout").unwrap().passed());
        let specs = backend.runtime.specs.lock().unwrap();
        assert!(!specs.is_empty());
        for s in specs.iter() {
            assert_eq!(s.image, "rust:1");
            assert_eq!(s.workdir.as_deref(), Some("/build"));
            // the repo is mounted at the configured workdir
            assert!(s.mounts.iter().any(|m| m.container == "/build"));
        }
    }

    #[test]
    fn secrets_spoofed_into_env_on_push() {
        let backend = backend(vec![0, 0]);
        let run = backend.execute(&plan(&wf_secret_effect()).unwrap()).unwrap();
        assert_eq!(run.unit("build").unwrap().secrets_spoofed, vec!["DEPLOY_TOKEN".to_string()]);
        let specs = backend.runtime.specs.lock().unwrap();
        let bs = specs.iter().find(|s| s.command.last().is_some_and(|c| c.contains("cargo build"))).unwrap();
        assert_eq!(bs.env.iter().find(|(k, _)| k == "DEPLOY_TOKEN").unwrap().1, "spoofed-DEPLOY_TOKEN");
    }

    #[test]
    fn secrets_withheld_on_fork_pr() {
        let backend = backend(vec![0, 0]);
        let plan = plan_for(&wf_secret_effect(), &EventContext::PullRequest { fork: true }).unwrap();
        let run = backend.execute(&plan).unwrap();
        let build = run.unit("build").unwrap();
        assert!(build.secrets_spoofed.is_empty());
        assert_eq!(build.secrets_withheld, vec!["DEPLOY_TOKEN".to_string()]);
        let specs = backend.runtime.specs.lock().unwrap();
        let bs = specs.iter().find(|s| s.command.last().is_some_and(|c| c.contains("cargo build"))).unwrap();
        assert!(bs.env.iter().all(|(k, _)| k != "DEPLOY_TOKEN"));
    }

    #[test]
    fn effects_and_transfers_recorded() {
        let run = backend(vec![0, 0]).execute(&plan(&wf_secret_effect()).unwrap()).unwrap();
        assert!(run.effect_fired("deploy"));
        assert_eq!(run.transfer_of("source"), Some(TransferKind::Copy));
    }

    #[test]
    fn failure_aborts_and_skips_remaining() {
        let run = backend(vec![1]).execute(&plan(&wf_secret_effect()).unwrap()).unwrap();
        assert_eq!(run.unit("checkout").unwrap().status, UnitStatus::Failed(1));
        assert_eq!(run.unit("build").unwrap().status, UnitStatus::Skipped);
    }

    #[test]
    fn workspace_and_store_mounted() {
        let backend = backend(vec![0, 0]);
        backend.execute(&plan(&wf_secret_effect()).unwrap()).unwrap();
        let specs = backend.runtime.specs.lock().unwrap();
        assert!(specs.iter().all(|s| s.mounts.iter().any(|m| m.container == "/workspace")));
        assert!(specs.iter().all(|s| s.mounts.iter().any(|m| m.container == "/loom-artifacts")));
    }
}

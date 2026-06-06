// pyo3's #[pymethods] expansion emits `.into()` on already-`PyErr` values;
// the lint fires on macro-generated code we don't control.
#![allow(clippy::useless_conversion)]

use ariadne::{Pipeline as RsPipeline, ir::Workflow};
use ariadne::backends::EmittingBackend;
use ariadne::backends::github::GithubActionsBackend;
use ariadne::backends::local::LocalBackend;
use ariadne::optimize::{optimize, OptLevel, OptimizeCtx};
use ariadne::analysis::Analysis;
use ariadne::profile::Profile;
use pyo3::exceptions::{PyValueError, PyRuntimeError};
use pyo3::prelude::*;

fn parse_workflow(json: &str) -> PyResult<Workflow> {
    serde_json::from_str(json).map_err(|e| PyValueError::new_err(format!("invalid TIR JSON: {e}")))
}

/// Parse optional profile JSON into a `Profile`; absent input is the empty
/// profile (the cost model then falls back to constants).
fn parse_profile(json: Option<&str>) -> PyResult<Profile> {
    match json {
        Some(s) => serde_json::from_str(s)
            .map_err(|e| PyValueError::new_err(format!("invalid profile JSON: {e}"))),
        None => Ok(Profile::default()),
    }
}

fn diags_to_strings(diags: &[ariadne::diagnostics::Diagnostic]) -> Vec<String> {
    diags.iter().map(|d| d.to_string()).collect()
}

/// An opaque handle to a compiled execution plan. Produced by Pipeline.plan()
/// and Pipeline.optimize(); consumed by Pipeline.emit().
#[pyclass(module = "ariadne_core")]
pub struct Plan {
    inner: ariadne::planner::Plan,
}

#[pymethods]
impl Plan {
    fn workflow_name(&self) -> &str { self.inner.workflow_name.as_str() }
    fn unit_count(&self) -> usize { self.inner.units.len() }
    fn max_concurrency(&self) -> usize { self.inner.max_concurrency() }

    fn diagnostics(&self) -> Vec<String> {
        diags_to_strings(&self.inner.diagnostics)
    }

    fn optimizations(&self) -> Vec<(String, String, String, String, String)> {
        self.inner.optimizations.iter().map(|o| {
            (o.pass.clone(), o.target.clone(), o.from.clone(), o.to.clone(), o.reason.clone())
        }).collect()
    }
}

/// Ariadne planning pipeline.
///
/// Construct from a TIR JSON string, then call validate(), plan(),
/// optimize(), and emit() in sequence.
#[pyclass(module = "ariadne_core")]
pub struct Pipeline {
    workflow: Workflow,
}

#[pymethods]
impl Pipeline {
    #[new]
    fn new(workflow_json: &str) -> PyResult<Self> {
        let workflow = parse_workflow(workflow_json)?;
        Ok(Pipeline { workflow })
    }

    /// Validate the workflow. Returns a list of diagnostic strings.
    /// Errors are prefixed with "error:", warnings with "warning:".
    fn validate(&self) -> Vec<String> {
        diags_to_strings(&RsPipeline::new(self.workflow.clone()).validate())
    }

    /// True if validate() would return any errors.
    fn has_errors(&self) -> bool {
        RsPipeline::new(self.workflow.clone())
            .validate()
            .iter()
            .any(|d| d.is_error())
    }

    /// Compute a baseline execution plan. Raises ValueError if planning fails.
    fn plan(&self) -> PyResult<Plan> {
        RsPipeline::new(self.workflow.clone())
            .plan()
            .map(|p| Plan { inner: p })
            .map_err(|errs| PyValueError::new_err(diags_to_strings(&errs).join("\n")))
    }

    /// Optimize a plan for the given backend and optimization level (0-3).
    /// `profile` is optional profile JSON (runner costs, durations, artifact
    /// sizes) that guides the cost model. Returns a new Plan with decisions.
    #[pyo3(signature = (plan, backend="local", level=2, profile=None))]
    fn optimize(&self, plan: &Plan, backend: &str, level: u8, profile: Option<&str>) -> PyResult<Plan> {
        if backend != "github" && backend != "local" {
            return Err(PyValueError::new_err(format!("unknown backend '{backend}'; supported: github, local")));
        }
        let caps = ariadne::backends::derive_capability_profile_from_inventory(
            self.workflow.inventory.as_ref()
        );
        let prof = parse_profile(profile)?;
        let wf = &self.workflow;
        let analysis = Analysis::of(wf);
        let ctx = OptimizeCtx {
            workflow: wf,
            profile: &prof,
            backend_caps: caps,
            policy: &wf.policies,
            analysis: &analysis,
            objectives: wf.policies.objectives.clone(),
            level: OptLevel::from_u8(level),
        };
        Ok(Plan { inner: optimize(plan.inner.clone(), &ctx) })
    }

    /// Emit backend-specific configuration (YAML for github, Bash for local).
    #[pyo3(signature = (plan, backend="local"))]
    fn emit(&self, plan: &Plan, backend: &str) -> PyResult<String> {
        match backend {
            "github" => GithubActionsBackend::default()
                .emit(&plan.inner)
                .map_err(|errs| PyRuntimeError::new_err(diags_to_strings(&errs).join("\n"))),
            "local" => LocalBackend::podman()
                .emit(&plan.inner)
                .map_err(|errs| PyRuntimeError::new_err(diags_to_strings(&errs).join("\n"))),
            other => Err(PyValueError::new_err(format!("unknown backend '{other}'"))),
        }
    }

    /// Convenience: validate + plan + optimize + emit in one call.
    #[pyo3(signature = (backend="local", level=2, profile=None))]
    fn compile(&self, backend: &str, level: u8, profile: Option<&str>) -> PyResult<String> {
        if self.has_errors() {
            let errs = self.validate().into_iter()
                .filter(|s| s.starts_with("error:"))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(PyValueError::new_err(format!("workflow has errors:\n{errs}")));
        }
        let plan = self.plan()?;
        let plan = self.optimize(&plan, backend, level, profile)?;
        self.emit(&plan, backend)
    }

    /// Run a loom test suite (JSON) against this workflow. Each case is planned
    /// for its event, optimized, and its plan-level assertions are evaluated.
    /// Returns (case, assertion, status, detail) rows where status is
    /// "pass" | "fail" | "skip" ("skip" = execution-level, run via `loom test`).
    #[pyo3(signature = (suite_json, backend="github", level=2, profile=None))]
    fn run_tests(&self, suite_json: &str, backend: &str, level: u8, profile: Option<&str>)
        -> PyResult<Vec<(String, String, String, String)>>
    {
        use ariadne::testing::{check_plan, TestSuite};
        use ariadne::planner::plan_for;
        use ariadne::backends::derive_capability_profile_from_inventory;

        let suite: TestSuite = serde_json::from_str(suite_json)
            .map_err(|e| PyValueError::new_err(format!("invalid test suite JSON: {e}")))?;
        let wf = &self.workflow;
        let caps = derive_capability_profile_from_inventory(wf.inventory.as_ref());
        let prof = parse_profile(profile)?;
        let analysis = Analysis::of(wf);

        let mut out = Vec::new();
        for case in &suite.cases {
            let backend_id = case.backend.as_deref().unwrap_or(backend);
            let plan = plan_for(wf, &case.event)
                .map_err(|errs| PyValueError::new_err(diags_to_strings(&errs).join("\n")))?;
            let ctx = OptimizeCtx {
                workflow: wf,
                profile: &prof,
                backend_caps: caps,
                policy: &wf.policies,
                analysis: &analysis,
                objectives: wf.policies.objectives.clone(),
                level: OptLevel::from_u8(level),
            };
            let plan = optimize(plan, &ctx);
            let results = match backend_id {
                "github" => check_plan(&plan, &GithubActionsBackend::default(), &case.assertions),
                "local" => check_plan(&plan, &LocalBackend::podman(), &case.assertions),
                other => return Err(PyValueError::new_err(format!("unknown backend '{other}'"))),
            };
            for r in &results {
                let status = if r.passed { "pass" } else { "fail" };
                out.push((case.name.clone(), format!("{:?}", r.assertion), status.into(), r.detail.clone()));
            }
            // Execution-level assertions can't be decided from the plan; surface
            // them as skipped so they are visible but don't fail the suite.
            for a in case.assertions.iter().filter(|a| a.requires_execution()) {
                out.push((
                    case.name.clone(),
                    format!("{a:?}"),
                    "skip".into(),
                    "execution-level assertion; run via `loom test`".into(),
                ));
            }
        }
        Ok(out)
    }
}

#[pymodule]
fn ariadne_core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Pipeline>()?;
    m.add_class::<Plan>()?;
    Ok(())
}

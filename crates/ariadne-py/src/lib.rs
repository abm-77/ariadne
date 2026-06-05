use ariadne::{Pipeline as RsPipeline, ir::Workflow};
use ariadne::backends::{BackendCapabilities, Backend};
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

fn diags_to_strings(diags: &[ariadne::diagnostics::Diagnostic]) -> Vec<String> {
    diags.iter().map(|d| d.to_string()).collect()
}

fn backend_caps(backend: &str) -> PyResult<BackendCapabilities> {
    match backend {
        "github" => Ok(GithubActionsBackend::default().capability_profile()),
        "local" => Ok(LocalBackend::podman().capability_profile()),
        other => Err(PyValueError::new_err(format!("unknown backend '{other}'; supported: github, local"))),
    }
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
    /// Returns a new Plan with optimization decisions applied.
    #[pyo3(signature = (plan, backend="local", level=2))]
    fn optimize(&self, plan: &Plan, backend: &str, level: u8) -> PyResult<Plan> {
        let caps = backend_caps(backend)?;
        let prof = Profile::default();
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
    #[pyo3(signature = (backend="local", level=2))]
    fn compile(&self, backend: &str, level: u8) -> PyResult<String> {
        if self.has_errors() {
            let errs = self.validate().into_iter()
                .filter(|s| s.starts_with("error:"))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(PyValueError::new_err(format!("workflow has errors:\n{errs}")));
        }
        let plan = self.plan()?;
        let plan = self.optimize(&plan, backend, level)?;
        self.emit(&plan, backend)
    }
}

#[pymodule]
fn ariadne_core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Pipeline>()?;
    m.add_class::<Plan>()?;
    Ok(())
}

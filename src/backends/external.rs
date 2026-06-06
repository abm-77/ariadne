//! External backend: discovers and communicates with an out-of-process backend
//! via a simple JSON stdin/stdout protocol.
//!
//! Protocol: the host writes a single-line JSON request to the process's stdin
//! and reads a single-line JSON response from stdout. Each operation is a
//! separate process invocation to keep the implementation stateless.
//!
//! Supported operations:
//!   describe -> returns id, workflow_capabilities
//!   emit     -> plan JSON in, rendered backend output out

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{Value as JsonValue, json};

use super::{
    BackendKind, Capability, Catalogue, EmittingBackend, InstructionSummary, WorkflowCapabilities,
    plan_to_request,
};
use crate::diagnostics::Diagnostic;
use crate::planner::{LogicalOp, Plan};

pub struct ExternalBackend {
    id: String,
    executable: PathBuf,
    workflow_caps: WorkflowCapabilities,
    catalogue: Catalogue,
}

impl ExternalBackend {
    /// Spawn the backend, call `describe`, and cache the metadata.
    pub fn load(id: &str, executable: PathBuf) -> Result<Self, String> {
        let desc = call(&executable, json!({"op": "describe"}))?;
        let workflow_caps = parse_workflow_caps(&desc["workflow_capabilities"]);
        Ok(Self {
            id: id.to_string(),
            executable,
            workflow_caps,
            catalogue: Catalogue::from_items(vec![]),
        })
    }
}

impl EmittingBackend for ExternalBackend {
    fn id(&self) -> &str {
        &self.id
    }
    fn backend_kind(&self) -> BackendKind {
        BackendKind::Custom(self.id.clone())
    }
    fn capabilities(&self) -> Vec<Capability> {
        vec![]
    }
    fn workflow_capabilities(&self) -> WorkflowCapabilities {
        self.workflow_caps
    }
    fn catalogue(&self) -> &Catalogue {
        &self.catalogue
    }

    fn emit(&self, plan: &Plan) -> Result<String, Vec<Diagnostic>> {
        let req = plan_to_request(plan);
        let plan_json = serde_json::to_value(&req).map_err(|e| {
            vec![Diagnostic::error(
                crate::diagnostics::DiagCode::UnknownAction,
                format!("serialize plan: {e}"),
            )]
        })?;
        let response =
            call(&self.executable, json!({"op": "emit", "plan": plan_json})).map_err(|e| {
                vec![Diagnostic::error(
                    crate::diagnostics::DiagCode::UnknownAction,
                    format!("external backend: {e}"),
                )]
            })?;
        response["output"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| {
                vec![Diagnostic::error(
                    crate::diagnostics::DiagCode::UnknownAction,
                    "external backend: missing 'output' field in response",
                )]
            })
    }

    fn select_op(&self, _op: &LogicalOp) -> Option<InstructionSummary> {
        None
    }
}

fn call(executable: &PathBuf, request: JsonValue) -> Result<JsonValue, String> {
    let request_line =
        serde_json::to_string(&request).map_err(|e| format!("serialize request: {e}"))?;

    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn '{}': {e}", executable.display()))?;

    child
        .stdin
        .take()
        .unwrap()
        .write_all(request_line.as_bytes())
        .map_err(|e| format!("write stdin: {e}"))?;

    let output = child.wait_with_output().map_err(|e| format!("wait: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("exited with {}: {}", output.status, stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim())
        .map_err(|e| format!("parse response: {e} (got: {})", stdout.trim()))
}

fn parse_workflow_caps(v: &JsonValue) -> WorkflowCapabilities {
    let mut caps = WorkflowCapabilities::empty();
    if let Some(arr) = v.as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                match s {
                    "jobs" => caps |= WorkflowCapabilities::JOBS,
                    "dependencies" => caps |= WorkflowCapabilities::DEPENDENCIES,
                    "conditions" => caps |= WorkflowCapabilities::CONDITIONS,
                    "matrices" => caps |= WorkflowCapabilities::MATRICES,
                    "permissions" => caps |= WorkflowCapabilities::PERMISSIONS,
                    "secrets" => caps |= WorkflowCapabilities::SECRETS,
                    "approvals" => caps |= WorkflowCapabilities::APPROVALS,
                    "runner_selection" => caps |= WorkflowCapabilities::RUNNER_SELECTION,
                    _ => {}
                }
            }
        }
    }
    caps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_caps_parse_from_string_array() {
        let v = json!(["jobs", "approvals", "secrets"]);
        let caps = parse_workflow_caps(&v);
        assert!(caps.contains(WorkflowCapabilities::JOBS));
        assert!(caps.contains(WorkflowCapabilities::APPROVALS));
        assert!(!caps.contains(WorkflowCapabilities::MATRICES));
    }

    #[test]
    fn unknown_capability_strings_ignored() {
        let v = json!(["jobs", "unknown_capability"]);
        let caps = parse_workflow_caps(&v);
        assert!(caps.contains(WorkflowCapabilities::JOBS));
        assert!(!caps.contains(WorkflowCapabilities::SECRETS));
    }
}

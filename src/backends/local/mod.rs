mod exec;
mod instructions;
pub mod runtime;

use crate::backends::{Backend, BackendKind, Capability, Catalogue, Instruction, Renderer};
use crate::planner::{LogicalOp, Plan};
pub use crate::backends::renderers::{BashRenderer, BashScript, BashUnit};
pub use runtime::{ContainerRuntime, ContainerSpec, DockerRuntime, Mount, PodmanRuntime, RunResult, RuntimeError};

pub struct LocalOptions {
    pub default_image: String,
    pub workdir: String,
}

impl Default for LocalOptions {
    fn default() -> Self {
        Self { default_image: "ubuntu:24.04".into(), workdir: "/workspace".into() }
    }
}

pub struct LocalBackend<R: ContainerRuntime> {
    pub runtime: R,
    pub opts: LocalOptions,
    catalogue: Catalogue,
}

impl<R: ContainerRuntime> LocalBackend<R> {
    pub fn new(runtime: R) -> Self {
        Self { runtime, opts: LocalOptions::default(), catalogue: instructions::catalogue() }
    }

    /// Container image actions run in (default `ubuntu:24.04`). Use one that
    /// ships the tools your scripts need, e.g. `rust:1` for git + cargo.
    pub fn with_image(mut self, image: impl Into<String>) -> Self {
        self.opts.default_image = image.into();
        self
    }

    /// Working directory the host repo is mounted at inside the container
    /// (default `/workspace`).
    pub fn with_workdir(mut self, workdir: impl Into<String>) -> Self {
        self.opts.workdir = workdir.into();
        self
    }
}

impl LocalBackend<PodmanRuntime> {
    pub fn podman() -> Self { Self::new(PodmanRuntime::default()) }
}

impl LocalBackend<DockerRuntime> {
    pub fn docker() -> Self { Self::new(DockerRuntime::default()) }
}

impl<R: ContainerRuntime> Backend for LocalBackend<R> {
    type Ir = BashScript;
    type Options = LocalOptions;

    fn name(&self) -> &str { "local" }
    fn backend_kind(&self) -> BackendKind { BackendKind::Local }
    fn capabilities(&self) -> Vec<Capability> { instructions::capabilities() }
    fn catalogue(&self) -> &Catalogue { &self.catalogue }
    fn options(&self) -> &LocalOptions { &self.opts }

    fn workflow_capabilities(&self) -> crate::backends::WorkflowCapabilities {
        use crate::backends::WorkflowCapabilities as WC;
        WC::JOBS | WC::DEPENDENCIES | WC::SECRETS | WC::APPROVALS
    }

    fn lower(&self, plan: &Plan) -> BashScript {
        let selector = self.selector();
        let caps = self.capabilities();
        let units = plan.units.iter().map(|unit| {
            let lines: Vec<String> = unit.ops.iter()
                .flat_map(|op| {
                    selector.select(op, &caps, &[])
                        .map(|sel| lower_op(op, sel.instruction))
                        .unwrap_or_default()
                })
                .collect();
            BashUnit { label: unit.action_name.to_string(), lines }
        }).collect();
        BashScript { units }
    }

    fn render(&self, ir: &BashScript) -> String {
        BashRenderer.render(ir)
    }
}

pub(crate) fn lower_op(op: &LogicalOp, instr: &Instruction) -> Vec<String> {
    let kind = instr.implementation.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "process.exec" => match op {
            LogicalOp::RunShell { script, .. } => vec![script.clone()],
            LogicalOp::CheckoutRepo => {
                let argv: Vec<&str> = instr.implementation.get("argv")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                if argv.is_empty() { vec![] } else { vec![argv.join(" ")] }
            }
            _ => vec![],
        },
        "local.copy" => match op {
            // Archive the artifact into the mock store by its path. If the path
            // is absent (build didn't produce it), drop a placeholder so the
            // transfer is still observable. Writes only to the store, never the
            // workspace.
            LogicalOp::UploadArtifact { name, path: Some(p), .. } => vec![format!(
                "mkdir -p \"$LOOM_ARTIFACT_STORE/{name}\"; \
                 cp -r \"{p}\" \"$LOOM_ARTIFACT_STORE/{name}/\" 2>/dev/null \
                 || echo \"mock:{name}\" > \"$LOOM_ARTIFACT_STORE/{name}/.mock\""
            )],
            _ => vec![],
        },
        "local.cache" => {
            let action = instr.implementation.get("action").and_then(|v| v.as_str()).unwrap_or("");
            match (action, op) {
                ("restore", LogicalOp::RestoreCache { key }) => vec![format!(
                    "if [ -f \"$LOOM_CACHE_STORE/{key}.tar\" ]; then \
                     tar -xf \"$LOOM_CACHE_STORE/{key}.tar\" -C .; \
                     fi"
                )],
                ("save", LogicalOp::SaveCache { key }) => vec![format!(
                    "mkdir -p \"$LOOM_CACHE_STORE\"; \
                     tar -cf \"$LOOM_CACHE_STORE/{key}.tar\" .cache/{key} 2>/dev/null || true"
                )],
                _ => vec![],
            }
        }
        "local.prompt" => match op {
            LogicalOp::RequestApproval { reason } => vec![
                format!("read -r -p 'Approval required: {reason} [y/N] ' _loom_approval"),
                r#"[ "$_loom_approval" = "y" ] || { echo "Approval denied"; exit 1; }"#.into(),
            ],
            _ => vec![],
        },
        "local.noop" => vec![],
        "local.native" => match op {
            LogicalOp::Native { fallback, .. } => vec![fallback.clone()],
            _ => vec![],
        },
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::Selector;
    use crate::ir::{ArtifactType, WorkflowBuilder};

    fn simple_plan() -> Plan {
        let mut b = WorkflowBuilder::new("test");
        let src = b.artifact("source", ArtifactType::SourceTree);
        let bin = b.artifact("binary", ArtifactType::Binary);
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        b.shell_action("build", "build", &[src], &[bin], "cargo build --release");
        b.actor("local", &["ubuntu-latest"], &[]);
        crate::planner::plan(&b.build()).expect("plan failed")
    }

    #[test]
    fn emit_produces_bash_script() {
        let script = LocalBackend::podman().emit(&simple_plan()).unwrap();
        assert!(script.starts_with("#!/usr/bin/env bash"));
        assert!(script.contains("git checkout ."));
        assert!(script.contains("cargo build --release"));
    }

    #[test]
    fn emit_preserves_topological_order() {
        let script = LocalBackend::podman().emit(&simple_plan()).unwrap();
        let checkout_pos = script.find("git checkout .").unwrap();
        let build_pos = script.find("cargo build --release").unwrap();
        assert!(checkout_pos < build_pos);
    }

    #[test]
    fn backend_name_is_local() {
        assert_eq!(LocalBackend::podman().name(), "local");
    }

    #[test]
    fn catalogue_selects_correct_instructions() {
        let backend = LocalBackend::podman();
        let sel = Selector::for_backend(&backend);
        let caps = instructions::capabilities();
        assert!(sel.select(&LogicalOp::CheckoutRepo, &caps, &[]).is_some());
        let run = LogicalOp::RunShell { label: "x".into(), script: "echo hi".into(), env: Default::default() };
        assert!(sel.select(&run, &caps, &[]).is_some());
    }

    #[test]
    fn backend_trait_reports_kind_and_capabilities() {
        let backend = LocalBackend::podman();
        assert_eq!(backend.backend_kind(), BackendKind::Local);
        assert!(backend.capabilities().contains(&Capability::new("process.exec")));
    }

    #[test]
    fn bash_renderer_formats_header_and_sections() {
        let script = BashScript {
            units: vec![BashUnit { label: "build".into(), lines: vec!["cargo build".into()] }],
        };
        let out = BashRenderer.render(&script);
        assert!(out.contains("# build\ncargo build"));
    }

    #[test]
    fn catalogue_has_cache_and_approval_instructions() {
        let cat = instructions::catalogue();
        let ids: Vec<_> = cat.all().iter().map(|i| i.id.0.as_str()).collect();
        assert!(ids.contains(&"local.cache.restore"));
        assert!(ids.contains(&"local.cache.save"));
        assert!(ids.contains(&"local.approval.gate"));
    }

    #[test]
    fn cache_restore_emits_conditional_tar_extract() {
        let backend = LocalBackend::podman();
        let sel = Selector::for_backend(&backend);
        let caps = instructions::capabilities();
        let op = LogicalOp::RestoreCache { key: "cargo".into() };
        let selected = sel.select(&op, &caps, &[]).unwrap();
        let lines = lower_op(&op, selected.instruction);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("tar -xf"));
        assert!(lines[0].contains("cargo.tar"));
    }

    #[test]
    fn cache_save_emits_tar_create() {
        let backend = LocalBackend::podman();
        let sel = Selector::for_backend(&backend);
        let caps = instructions::capabilities();
        let op = LogicalOp::SaveCache { key: "cargo".into() };
        let selected = sel.select(&op, &caps, &[]).unwrap();
        let lines = lower_op(&op, selected.instruction);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("tar -cf"));
        assert!(lines[0].contains("cargo.tar"));
    }

    #[test]
    fn approval_gate_emits_read_prompt_and_guard() {
        let backend = LocalBackend::podman();
        let sel = Selector::for_backend(&backend);
        let caps = instructions::capabilities();
        let op = LogicalOp::RequestApproval { reason: "deploy to prod".into() };
        let selected = sel.select(&op, &caps, &[]).unwrap();
        let lines = lower_op(&op, selected.instruction);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("read"));
        assert!(lines[0].contains("deploy to prod"));
        assert!(lines[1].contains("exit 1"));
    }
}

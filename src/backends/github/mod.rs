mod instructions;

use crate::backends::{Backend, BackendKind, Capability, Catalogue, Instruction, Renderer, Selector};
use indexmap::IndexMap;
use crate::planner::{PhysicalOp, Plan};
use crate::backends::renderers::YamlRenderer;
use serde::Serialize;

pub struct EmitOptions {
    pub push_branches: Vec<String>,
    pub pull_request: bool,
}

impl Default for EmitOptions {
    fn default() -> Self {
        Self { push_branches: vec!["main".into()], pull_request: true }
    }
}

pub struct GithubActionsBackend {
    pub opts: EmitOptions,
    catalogue: Catalogue,
}

impl Default for GithubActionsBackend {
    fn default() -> Self {
        Self {
            opts: EmitOptions::default(),
            catalogue: instructions::catalogue(),
        }
    }
}

impl Backend for GithubActionsBackend {
    type Ir = GhWorkflow;
    type Options = EmitOptions;

    fn name(&self) -> &str { "github-actions" }
    fn backend_kind(&self) -> BackendKind { BackendKind::Github }
    fn capabilities(&self) -> Vec<Capability> { instructions::capabilities() }
    fn catalogue(&self) -> &Catalogue { &self.catalogue }
    fn options(&self) -> &EmitOptions { &self.opts }

    fn capability_profile(&self) -> crate::backends::BackendCapabilities {
        // Hosted GitHub: artifacts + cache, but no cross-job mounts / colocation.
        crate::backends::BackendCapabilities::CACHE
    }

    fn lower(&self, plan: &Plan) -> GhWorkflow {
        let selector = self.selector();
        let caps = self.capabilities();
        plan_to_gha(plan, &self.opts, &selector, &caps)
    }

    fn render(&self, ir: &GhWorkflow) -> String {
        YamlRenderer::default().render(ir)
    }
}

pub fn emit(plan: &Plan, opts: &EmitOptions) -> String {
    let backend = GithubActionsBackend {
        opts: EmitOptions {
            push_branches: opts.push_branches.clone(),
            pull_request: opts.pull_request,
        },
        catalogue: instructions::catalogue(),
    };
    backend.emit(plan).expect("emit failed")
}

fn lower_op(op: &PhysicalOp, instr: &Instruction) -> Vec<GhStep> {
    let kind = instr.implementation.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "github.uses" => {
            let uses_ref = instr.implementation.get("ref").and_then(|v| v.as_str()).unwrap_or("");
            let (step_name, with) = match op {
                PhysicalOp::UploadArtifact { name, path } => {
                    let mut m = IndexMap::new();
                    m.insert("name".into(), name.to_string());
                    m.insert("path".into(), path.clone().unwrap_or_else(|| name.to_string()));
                    (format!("Upload artifact {name}"), Some(m))
                }
                PhysicalOp::DownloadArtifact { name, .. } => {
                    let mut m = IndexMap::new();
                    m.insert("name".into(), name.to_string());
                    (format!("Download artifact {name}"), Some(m))
                }
                _ => {
                    let name = instr.implementation.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(instr.id.0.as_str())
                        .to_string();
                    (name, None)
                }
            };
            vec![GhStep { name: step_name, uses: Some(uses_ref.into()), with, ..Default::default() }]
        }
        "github.run" => {
            let PhysicalOp::RunShell { label, script, env } = op else { return vec![] };
            let mut sorted_env: Vec<_> = env.iter().collect();
            sorted_env.sort_by_key(|(k, _)| k.as_str());
            let env_map: IndexMap<_, _> = sorted_env.into_iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            vec![GhStep {
                name: format!("Run {label}"),
                run: Some(script.clone()),
                env: if env_map.is_empty() { None } else { Some(env_map) },
                ..Default::default()
            }]
        }
        _ => vec![],
    }
}

fn plan_to_gha(plan: &Plan, opts: &EmitOptions, selector: &Selector, backend_caps: &[Capability]) -> GhWorkflow {
    let on = GhOn {
        push: if opts.push_branches.is_empty() {
            None
        } else {
            Some(GhPush { branches: opts.push_branches.clone() })
        },
        pull_request: if opts.pull_request { Some(PullRequestTrigger) } else { None },
    };

    let jobs = plan.units.iter().map(|unit| {
        let steps: Vec<GhStep> = unit.ops.iter()
            .flat_map(|op| {
                selector.select(op, backend_caps, &[])
                    .map(|sel| lower_op(op, sel.instruction))
                    .unwrap_or_default()
            })
            .collect();

        (unit.id.to_string(), GhJob {
            runs_on: unit.runner.to_string(),
            needs: unit.needs.iter().map(|n| n.to_string()).collect(),
            steps,
        })
    }).collect();

    GhWorkflow { name: plan.workflow_name.to_string(), on, jobs }
}

#[derive(Debug, Clone, Serialize)]
pub struct GhWorkflow {
    pub name: String,
    pub on: GhOn,
    pub jobs: IndexMap<String, GhJob>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GhOn {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push: Option<GhPush>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequestTrigger>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GhPush {
    pub branches: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PullRequestTrigger;

impl Serialize for PullRequestTrigger {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        s.serialize_map(Some(0))?.end()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GhJob {
    #[serde(rename = "runs-on")]
    pub runs_on: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    pub steps: Vec<GhStep>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GhStep {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uses: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<IndexMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with: Option<IndexMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_request_trigger_serializes_as_empty_mapping() {
        assert_eq!(YamlRenderer::<PullRequestTrigger>::default().render(&PullRequestTrigger).trim(), "{}");
    }

    #[test]
    fn catalogue_has_standard_entries() {
        let cat = instructions::catalogue();
        let ids: Vec<_> = cat.entries().iter().map(|i| i.id.0.as_str()).collect();
        assert!(ids.contains(&"github.checkout.default"));
        assert!(ids.contains(&"github.shell.run"));
        assert!(ids.contains(&"github.artifact.upload"));
        assert!(ids.contains(&"github.artifact.download"));
    }

    #[test]
    fn checkout_op_selects_checkout_instruction() {
        let backend = GithubActionsBackend::default();
        let sel = Selector::for_backend(&backend);
        let caps = instructions::capabilities();
        let result = sel.select(&PhysicalOp::CheckoutRepo, &caps, &[]);
        assert_eq!(result.unwrap().instruction.id.0, "github.checkout.default");
    }

    #[test]
    fn render_checkout_produces_uses_step() {
        let backend = GithubActionsBackend::default();
        let sel = Selector::for_backend(&backend);
        let caps = instructions::capabilities();
        let selected = sel.select(&PhysicalOp::CheckoutRepo, &caps, &[]).unwrap();
        let steps = lower_op(&PhysicalOp::CheckoutRepo, selected.instruction);
        assert_eq!(steps[0].uses.as_deref(), Some("actions/checkout@v4"));
        assert_eq!(steps[0].name, "Checkout repository");
    }

    #[test]
    fn render_upload_includes_with_fields() {
        let backend = GithubActionsBackend::default();
        let sel = Selector::for_backend(&backend);
        let caps = instructions::capabilities();
        let op = PhysicalOp::UploadArtifact { name: "my-binary".into(), path: Some("dist/app".into()) };
        let selected = sel.select(&op, &caps, &[]).unwrap();
        let steps = lower_op(&op, selected.instruction);
        let with = steps[0].with.as_ref().unwrap();
        assert_eq!(with["name"], "my-binary");
        assert_eq!(with["path"], "dist/app");
    }

    #[test]
    fn backend_trait_reports_kind_and_capabilities() {
        let backend = GithubActionsBackend::default();
        assert_eq!(backend.backend_kind(), BackendKind::Github);
        let caps = backend.capabilities();
        assert!(caps.contains(&Capability::new("github.actions.uses")));
        assert!(caps.contains(&Capability::new("process.exec")));
    }
}

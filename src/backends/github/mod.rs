mod collect;
mod instructions;

pub use collect::GithubCollector;

use crate::backends::renderers::YamlRenderer;
use crate::backends::{
    Backend, BackendKind, Capability, Catalogue, Instruction, Renderer, Selector,
};
use crate::planner::{LogicalOp, Plan};
use indexmap::IndexMap;
use serde::Serialize;

pub struct EmitOptions {
    pub push_branches: Vec<String>,
    pub pull_request: bool,
}

impl Default for EmitOptions {
    fn default() -> Self {
        Self {
            push_branches: vec!["main".into()],
            pull_request: true,
        }
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

    fn name(&self) -> &str {
        "github"
    }
    fn backend_kind(&self) -> BackendKind {
        BackendKind::Github
    }
    fn capabilities(&self) -> Vec<Capability> {
        instructions::capabilities()
    }
    fn catalogue(&self) -> &Catalogue {
        &self.catalogue
    }
    fn options(&self) -> &EmitOptions {
        &self.opts
    }

    fn workflow_capabilities(&self) -> crate::backends::WorkflowCapabilities {
        use crate::backends::WorkflowCapabilities as WC;
        WC::JOBS
            | WC::DEPENDENCIES
            | WC::CONDITIONS
            | WC::MATRICES
            | WC::PERMISSIONS
            | WC::SECRETS
            | WC::APPROVALS
            | WC::RUNNER_SELECTION
    }

    fn lower(&self, plan: &Plan) -> GhWorkflow {
        let selector = self.selector();
        let mut caps = self.capabilities();
        // Inventory-derived implementation capabilities let the catalogue decide
        // whether to upgrade a Native op to a native step at emit time.
        caps.extend(plan.impl_capabilities.iter().map(Capability::new));
        plan_to_gha(plan, &self.opts, &selector, &caps)
    }

    fn render(&self, ir: &GhWorkflow) -> String {
        YamlRenderer::default().render(ir)
    }

    fn emit(&self, plan: &Plan) -> Result<String, Vec<crate::diagnostics::Diagnostic>> {
        // Render, then annotate fused jobs with the actions they ran.
        Ok(annotate_fused_jobs(self.render(&self.lower(plan)), plan))
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

fn lower_op(op: &LogicalOp, instr: &Instruction) -> Vec<GhStep> {
    let kind = instr
        .implementation
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match kind {
        "github.uses" => {
            let uses_ref = instr
                .implementation
                .get("ref")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let (step_name, with) = match op {
                LogicalOp::UploadArtifact {
                    name,
                    path,
                    lifetime,
                } => {
                    let mut m = IndexMap::new();
                    m.insert("name".into(), name.to_string());
                    m.insert(
                        "path".into(),
                        path.clone().unwrap_or_else(|| name.to_string()),
                    );
                    if let Some(days) = lifetime.as_deref().and_then(retention_days) {
                        m.insert("retention-days".into(), days.to_string());
                    }
                    (format!("Upload artifact {name}"), Some(m))
                }
                LogicalOp::DownloadArtifact { name, .. } => {
                    let mut m = IndexMap::new();
                    m.insert("name".into(), name.to_string());
                    (format!("Download artifact {name}"), Some(m))
                }
                LogicalOp::RestoreCache { key } => {
                    let mut m = IndexMap::new();
                    m.insert("key".into(), key.to_string());
                    m.insert("path".into(), format!(".cache/{key}"));
                    (format!("Restore cache {key}"), Some(m))
                }
                LogicalOp::SaveCache { key } => {
                    let mut m = IndexMap::new();
                    m.insert("key".into(), key.to_string());
                    m.insert("path".into(), format!(".cache/{key}"));
                    (format!("Save cache {key}"), Some(m))
                }
                LogicalOp::Native { id, args, .. } => {
                    let with = if args.is_empty() {
                        None
                    } else {
                        Some(args.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    };
                    let name = instr
                        .implementation
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("{id} ({uses_ref})"));
                    (name, with)
                }
                _ => {
                    let name = instr
                        .implementation
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(instr.id.0.as_str())
                        .to_string();
                    (name, None)
                }
            };
            vec![GhStep {
                name: step_name,
                uses: Some(uses_ref.into()),
                with,
                ..Default::default()
            }]
        }
        "github.run.native" => {
            let LogicalOp::Native { id, fallback, .. } = op else {
                return vec![];
            };
            vec![GhStep {
                name: format!("Run {id}"),
                run: Some(fallback.clone()),
                ..Default::default()
            }]
        }
        "github.run" => {
            let LogicalOp::RunShell { label, script, env } = op else {
                return vec![];
            };
            let mut sorted_env: Vec<_> = env.iter().collect();
            sorted_env.sort_by_key(|(k, _)| k.as_str());
            let env_map: IndexMap<_, _> = sorted_env
                .into_iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            vec![GhStep {
                name: format!("Run {label}"),
                run: Some(script.clone()),
                env: if env_map.is_empty() {
                    None
                } else {
                    Some(env_map)
                },
                ..Default::default()
            }]
        }
        "github.environment" => {
            // Enforcement happens at the job level via `environment:`. The step
            // is a visible marker; the real gate is the deployment protection rule.
            let LogicalOp::RequestApproval { reason } = op else {
                return vec![];
            };
            vec![GhStep {
                name: format!("Approval gate: {reason}"),
                run: Some(format!("echo 'This job requires approval: {reason}'")),
                ..Default::default()
            }]
        }
        _ => vec![],
    }
}

fn unit_environment(ops: &[LogicalOp]) -> Option<String> {
    ops.iter().find_map(|op| {
        if let LogicalOp::RequestApproval { reason } = op {
            Some(reason.clone())
        } else {
            None
        }
    })
}

/// Native setup steps for the unit's toolchains (e.g. actions/setup-python),
/// using the version/channel declared in the inventory. Empty unless the
/// workflow opts into provisioning its environment.
fn setup_steps(plan: &Plan, unit: &crate::planner::ExecutionUnit) -> Vec<GhStep> {
    if !plan.install_dependencies {
        return Vec::new();
    }
    unit.toolchains
        .iter()
        .filter_map(|tc| {
            let version = plan.toolchains.get(tc.as_str()).cloned().flatten();
            match tc.as_str() {
                "python" => Some(setup_uses(
                    "Set up Python",
                    "actions/setup-python@v5",
                    "python-version",
                    version.unwrap_or_else(|| "3.x".into()),
                )),
                "node" => Some(setup_uses(
                    "Set up Node",
                    "actions/setup-node@v4",
                    "node-version",
                    version.unwrap_or_else(|| "lts/*".into()),
                )),
                "go" => Some(setup_uses(
                    "Set up Go",
                    "actions/setup-go@v5",
                    "go-version",
                    version.unwrap_or_else(|| "stable".into()),
                )),
                // dtolnay/rust-toolchain selects the channel via the action ref.
                "rust" => Some(GhStep {
                    name: "Set up Rust".into(),
                    uses: Some(format!(
                        "dtolnay/rust-toolchain@{}",
                        version.unwrap_or_else(|| "stable".into())
                    )),
                    ..Default::default()
                }),
                _ => None,
            }
        })
        .collect()
}

/// The original action names a fused unit ran, or None for a single action. A
/// fused unit's action name is the parts joined with `-and-`.
fn fused_actions(action_name: &str) -> Option<Vec<String>> {
    let parts: Vec<String> = action_name.split("-and-").map(str::to_string).collect();
    (parts.len() > 1).then_some(parts)
}

/// A GitHub job id for a unit. A single action keeps its (kebab) name; a fused
/// unit gets a short, stable `fused-<hash>` id (the actions it ran are listed in
/// a comment above the job), since concatenating every name overruns the id limit.
fn job_key(action_name: &str) -> String {
    if fused_actions(action_name).is_some() {
        format!("fused-{}", short_hash(action_name))
    } else {
        action_name.to_lowercase().replace([' ', '_', '+'], "-")
    }
}

fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", h & 0xffff_ffff)
}

/// Insert a `# fuses: a, b, c` comment above each fused job so the actions a
/// `fused-<hash>` job ran stay visible in the emitted YAML.
fn annotate_fused_jobs(yaml: String, plan: &Plan) -> String {
    let comments: std::collections::HashMap<String, String> = plan
        .units
        .iter()
        .filter_map(|u| {
            fused_actions(&u.action_name).map(|acts| {
                (
                    job_key(&u.action_name),
                    format!("fuses: {}", acts.join(", ")),
                )
            })
        })
        .collect();
    if comments.is_empty() {
        return yaml;
    }
    let mut out = String::with_capacity(yaml.len());
    for line in yaml.lines() {
        if let Some(key) = line.strip_prefix("  ").and_then(|l| l.strip_suffix(':'))
            && let Some(comment) = comments.get(key)
        {
            out.push_str("  # ");
            out.push_str(comment);
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn setup_uses(name: &str, uses: &str, key: &str, value: String) -> GhStep {
    let mut with = IndexMap::new();
    with.insert(key.to_string(), value);
    GhStep {
        name: name.into(),
        uses: Some(uses.into()),
        with: Some(with),
        ..Default::default()
    }
}

/// A single "Install dependencies" step that runs the install command for each
/// of the unit's declared tools (already lowered to a manager command in the
/// plan's shared table), or None when install-on-start is off. Commands are
/// de-duplicated, since several tools can share one.
fn install_step(plan: &Plan, unit: &crate::planner::ExecutionUnit) -> Option<GhStep> {
    if !plan.install_dependencies || unit.dependencies.is_empty() {
        return None;
    }
    let mut cmds: Vec<String> = Vec::new();
    for tool in &unit.dependencies {
        if let Some(cmd) = plan.dependencies.get(tool.as_str())
            && !cmds.contains(cmd)
        {
            cmds.push(cmd.clone());
        }
    }
    if cmds.is_empty() {
        return None;
    }
    Some(GhStep {
        name: "Install dependencies".into(),
        run: Some(cmds.join("\n")),
        ..Default::default()
    })
}

/// Map a semantic artifact lifetime to GitHub `retention-days`. GitHub's
/// retention is coarse: whole days, minimum 1, maximum 90. A sub-day requirement
/// (e.g. "12h", "30m") rounds UP to 1 day, the finest GitHub can express, which
/// over-satisfies the requirement legally. Returns None for the workflow
/// default. Categories: ephemeral=1, release/permanent=90, workflow=default.
fn retention_days(lifetime: &str) -> Option<u32> {
    let days: u64 = match lifetime {
        "ephemeral" => 1,
        "release" | "permanent" => 90,
        "workflow" => return None,
        other => {
            let secs = humantime::parse_duration(other).ok()?.as_secs();
            // ceil to whole days; GitHub cannot retain for less than a day.
            secs.div_ceil(86_400).max(1)
        }
    };
    Some(days.clamp(1, 90) as u32)
}

/// Build the GitHub `on:` block from the plan's triggers. An empty trigger list
/// falls back to the backend's default (push to configured branches + PRs) so
/// existing behavior is preserved when a workflow declares no triggers.
fn build_on(triggers: &[crate::ir::Trigger], opts: &EmitOptions) -> GhOn {
    use crate::ir::Trigger;
    if triggers.is_empty() {
        return GhOn {
            push: if opts.push_branches.is_empty() {
                None
            } else {
                Some(GhPush {
                    branches: opts.push_branches.clone(),
                    tags: vec![],
                })
            },
            pull_request: if opts.pull_request {
                Some(EmptyMapping)
            } else {
                None
            },
            schedule: None,
            workflow_dispatch: None,
        };
    }
    let (mut branches, mut tags, mut crons) = (Vec::new(), Vec::new(), Vec::new());
    let (mut pr, mut manual) = (false, false);
    for t in triggers {
        match t {
            Trigger::PullRequest => pr = true,
            Trigger::Push { branches: b } => branches.extend(b.iter().cloned()),
            Trigger::Tag { pattern } => tags.push(pattern.clone()),
            Trigger::Schedule { cron } => crons.push(GhSchedule { cron: cron.clone() }),
            Trigger::Manual => manual = true,
        }
    }
    GhOn {
        push: if branches.is_empty() && tags.is_empty() {
            None
        } else {
            Some(GhPush { branches, tags })
        },
        pull_request: if pr { Some(EmptyMapping) } else { None },
        schedule: if crons.is_empty() { None } else { Some(crons) },
        workflow_dispatch: if manual { Some(EmptyMapping) } else { None },
    }
}

fn plan_to_gha(
    plan: &Plan,
    opts: &EmitOptions,
    selector: &Selector,
    backend_caps: &[Capability],
) -> GhWorkflow {
    let on = build_on(&plan.triggers, opts);

    // The job id is the (kebab-cased) action name, so a fused unit's job is named
    // for all the actions it ran, capped to GitHub's id length. `needs` reference
    // these same keys; the full name goes in the job's display `name`.
    let job_id: std::collections::HashMap<ustr::Ustr, String> = plan
        .units
        .iter()
        .map(|u| (u.id, job_key(&u.action_name)))
        .collect();

    let jobs = plan
        .units
        .iter()
        .map(|unit| {
            let mut steps: Vec<GhStep> = unit
                .ops
                .iter()
                .flat_map(|op| {
                    selector
                        .select(op, backend_caps, &[])
                        .map(|sel| lower_op(op, sel.instruction))
                        .unwrap_or_default()
                })
                .collect();

            // Optionally provision the unit's toolchains and install its declared
            // tools on job start (after checkout). Without this opt-in the
            // environment is assumed to provide them. Toolchain setup precedes
            // tool install (e.g. set up Python before `pip install`).
            let after = steps
                .iter()
                .position(|s| s.uses.as_deref().is_some_and(|u| u.contains("checkout")))
                .map_or(0, |i| i + 1);
            let mut prelude = setup_steps(plan, unit);
            prelude.extend(install_step(plan, unit));
            for (offset, step) in prelude.into_iter().enumerate() {
                steps.insert(after + offset, step);
            }

            (
                job_id[&unit.id].clone(),
                GhJob {
                    runs_on: unit.runner.to_string(),
                    needs: unit.needs.iter().map(|n| job_id[n].clone()).collect(),
                    environment: unit_environment(&unit.ops),
                    timeout_minutes: unit.timeout.as_deref().and_then(timeout_minutes),
                    concurrency: unit.coordination.as_ref().map(concurrency_of),
                    steps,
                },
            )
        })
        .collect();

    GhWorkflow {
        name: plan.workflow_name.to_string(),
        on,
        concurrency: plan.coordination.as_ref().map(concurrency_of),
        jobs,
    }
}

/// Map a duration (e.g. "30m", "1h") to GitHub `timeout-minutes`, rounded up to
/// at least 1 minute.
fn timeout_minutes(timeout: &str) -> Option<u32> {
    let secs = humantime::parse_duration(timeout).ok()?.as_secs();
    Some((secs.div_ceil(60).max(1)).min(u32::MAX as u64) as u32)
}

fn concurrency_of(c: &crate::ir::Coordination) -> GhConcurrency {
    GhConcurrency {
        group: c.group.clone(),
        cancel_in_progress: c.cancel_in_progress,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GhWorkflow {
    pub name: String,
    pub on: GhOn,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<GhConcurrency>,
    pub jobs: IndexMap<String, GhJob>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GhConcurrency {
    pub group: String,
    #[serde(rename = "cancel-in-progress")]
    pub cancel_in_progress: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GhOn {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push: Option<GhPush>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<EmptyMapping>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Vec<GhSchedule>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_dispatch: Option<EmptyMapping>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GhSchedule {
    pub cron: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GhPush {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Serializes as an empty YAML mapping `{}`. Used for `on:` entries that take no
/// configuration (`pull_request:`, `workflow_dispatch:`).
#[derive(Debug, Clone)]
pub struct EmptyMapping;

impl Serialize for EmptyMapping {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(rename = "timeout-minutes", skip_serializing_if = "Option::is_none")]
    pub timeout_minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<GhConcurrency>,
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
    use crate::ir::{ArtifactType, ConsequenceKind, WorkflowBuilder};
    use crate::planner::plan;

    #[test]
    fn pull_request_trigger_serializes_as_empty_mapping() {
        assert_eq!(
            YamlRenderer::<EmptyMapping>::default()
                .render(&EmptyMapping)
                .trim(),
            "{}"
        );
    }

    fn publish_workflow(impl_id: Option<&str>) -> crate::ir::Workflow {
        let impls = impl_id
            .map(|id| format!(r#","implementations":[{{"id":"{id}"}}]"#))
            .unwrap_or_default();
        let json = format!(
            r#"{{
                "name": "w",
                "action_calls": [{{"name": "publish", "action": "publish"}}],
                "action_defs": [{{"id": "publish", "implementations": [
                    {{"kind": "semantic", "op": "package.publish", "args": {{"dist": "dist/*.whl"}}}}
                ]}}],
                "inventory": {{"id": "i",
                    "actors": [{{"id": "r", "labels": ["ubuntu-latest"]}}]{impls}
                }}
            }}"#
        );
        serde_json::from_str(&json).expect("valid TIR")
    }

    #[test]
    fn triggers_drive_the_on_block() {
        use crate::ir::{ArtifactType, Trigger, WorkflowBuilder};
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::SourceTree);
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        b.actor("r", &["ubuntu-latest"], &[]);
        b.trigger(Trigger::Push {
            branches: vec!["main".into()],
        });
        b.trigger(Trigger::Tag {
            pattern: "v*".into(),
        });
        b.trigger(Trigger::Schedule {
            cron: "0 2 * * *".into(),
        });
        b.trigger(Trigger::Manual);
        let yaml = emit(&plan(&b.build()).unwrap(), &EmitOptions::default());
        assert!(yaml.contains("tags:"), "{yaml}");
        assert!(yaml.contains("- v*"), "{yaml}");
        assert!(yaml.contains("schedule:"), "{yaml}");
        assert!(yaml.contains("cron: 0 2 * * *"), "{yaml}");
        assert!(yaml.contains("workflow_dispatch:"), "{yaml}");
        assert!(
            !yaml.contains("pull_request:"),
            "no PR trigger declared: {yaml}"
        );
    }

    #[test]
    fn timeout_and_coordination_emit() {
        use crate::ir::{ArtifactType, Coordination, WorkflowBuilder};
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::SourceTree);
        let a = b.shell_action("deploy", "deploy", &[], &[src], "./deploy.sh");
        b.actor("r", &["ubuntu-latest"], &[]);
        let mut wf = b.build();
        wf.action_calls[a.idx()].timeout = Some("30m".into());
        wf.action_calls[a.idx()].coordination = Some(Coordination {
            group: "prod".into(),
            cancel_in_progress: false,
        });
        wf.coordination = Some(Coordination {
            group: "release".into(),
            cancel_in_progress: true,
        });
        let yaml = emit(&plan(&wf).unwrap(), &EmitOptions::default());
        assert!(yaml.contains("timeout-minutes: 30"), "{yaml}");
        assert!(yaml.contains("group: prod"), "{yaml}");
        assert!(yaml.contains("group: release"), "{yaml}");
        assert!(yaml.contains("cancel-in-progress: true"), "{yaml}");
    }

    #[test]
    fn artifact_lifetime_sets_retention_days() {
        use crate::ir::{ArtifactType, WorkflowBuilder};
        let mut b = WorkflowBuilder::new("w");
        let bin = b.artifact_at("bin", ArtifactType::Binary, "target/app");
        // Reach into the artifact to set a lifetime (builder has no sugar yet).
        let mut wf = {
            b.shell_action("build", "build", &[], &[bin], "make");
            b.actor("r", &["ubuntu-latest"], &[]);
            b.build()
        };
        wf.artifacts[bin.idx()].lifetime = Some("14d".into());
        let yaml = emit(&plan(&wf).unwrap(), &EmitOptions::default());
        assert!(
            yaml.contains("retention-days: '14'") || yaml.contains("retention-days: 14"),
            "{yaml}"
        );
    }

    #[test]
    fn sub_day_lifetime_rounds_up_to_one_day() {
        assert_eq!(retention_days("12h"), Some(1));
        assert_eq!(retention_days("30m"), Some(1));
        assert_eq!(retention_days("90s"), Some(1));
        assert_eq!(retention_days("2d"), Some(2));
        assert_eq!(retention_days("1w"), Some(7));
        assert_eq!(retention_days("365d"), Some(90)); // clamped to GitHub max
        assert_eq!(retention_days("workflow"), None);
        assert_eq!(retention_days("ephemeral"), Some(1));
    }

    #[test]
    fn no_triggers_falls_back_to_default_on_block() {
        use crate::ir::{ArtifactType, WorkflowBuilder};
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::SourceTree);
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        b.actor("r", &["ubuntu-latest"], &[]);
        let yaml = emit(&plan(&b.build()).unwrap(), &EmitOptions::default());
        assert!(yaml.contains("pull_request:"), "{yaml}");
        assert!(yaml.contains("branches:"), "{yaml}");
    }

    #[test]
    fn native_publish_upgrades_to_uses_when_inventory_permits() {
        let wf = publish_workflow(Some("pypa-publish-action"));
        let yaml = emit(&plan(&wf).unwrap(), &EmitOptions::default());
        assert!(
            yaml.contains("uses: pypa/gh-action-pypi-publish@release/v1"),
            "{yaml}"
        );
        assert!(!yaml.contains("twine upload"), "{yaml}");
    }

    #[test]
    fn native_publish_falls_back_to_shell_without_capability() {
        let wf = publish_workflow(Some("twine"));
        let yaml = emit(&plan(&wf).unwrap(), &EmitOptions::default());
        assert!(yaml.contains("run: twine upload dist/*.whl"), "{yaml}");
        assert!(!yaml.contains("pypa/gh-action"), "{yaml}");
    }

    #[test]
    fn catalogue_has_standard_entries() {
        let cat = instructions::catalogue();
        let ids: Vec<_> = cat.all().iter().map(|i| i.id.0.as_str()).collect();
        assert!(ids.contains(&"github.checkout.default"));
        assert!(ids.contains(&"github.shell.run"));
        assert!(ids.contains(&"github.artifact.upload"));
        assert!(ids.contains(&"github.artifact.download"));
        assert!(ids.contains(&"github.cache.restore"));
        assert!(ids.contains(&"github.cache.save"));
        assert!(ids.contains(&"github.approval.gate"));
    }

    #[test]
    fn checkout_op_selects_checkout_instruction() {
        let backend = GithubActionsBackend::default();
        let sel = Selector::for_backend(&backend);
        let caps = instructions::capabilities();
        let result = sel.select(&LogicalOp::CheckoutRepo, &caps, &[]);
        assert_eq!(result.unwrap().instruction.id.0, "github.checkout.default");
    }

    #[test]
    fn render_checkout_produces_uses_step() {
        let backend = GithubActionsBackend::default();
        let sel = Selector::for_backend(&backend);
        let caps = instructions::capabilities();
        let selected = sel.select(&LogicalOp::CheckoutRepo, &caps, &[]).unwrap();
        let steps = lower_op(&LogicalOp::CheckoutRepo, selected.instruction);
        assert_eq!(steps[0].uses.as_deref(), Some("actions/checkout@v4"));
        assert_eq!(steps[0].name, "Checkout repository");
    }

    #[test]
    fn render_upload_includes_with_fields() {
        let backend = GithubActionsBackend::default();
        let sel = Selector::for_backend(&backend);
        let caps = instructions::capabilities();
        let op = LogicalOp::UploadArtifact {
            name: "my-binary".into(),
            path: Some("dist/app".into()),
            lifetime: None,
        };
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
        assert!(caps.contains(&Capability::new("github.action_calls.uses")));
        assert!(caps.contains(&Capability::new("process.exec")));
    }

    #[test]
    fn cache_restore_emits_uses_step_with_key_and_path() {
        let backend = GithubActionsBackend::default();
        let sel = Selector::for_backend(&backend);
        let caps = instructions::capabilities();
        let op = LogicalOp::RestoreCache {
            key: "cargo".into(),
        };
        let selected = sel.select(&op, &caps, &[]).unwrap();
        let steps = lower_op(&op, selected.instruction);
        assert_eq!(steps[0].uses.as_deref(), Some("actions/cache@v4"));
        let with = steps[0].with.as_ref().unwrap();
        assert_eq!(with["key"], "cargo");
        assert_eq!(with["path"], ".cache/cargo");
    }

    #[test]
    fn cache_save_emits_uses_step_with_key_and_path() {
        let backend = GithubActionsBackend::default();
        let sel = Selector::for_backend(&backend);
        let caps = instructions::capabilities();
        let op = LogicalOp::SaveCache {
            key: "cargo".into(),
        };
        let selected = sel.select(&op, &caps, &[]).unwrap();
        let steps = lower_op(&op, selected.instruction);
        assert_eq!(steps[0].uses.as_deref(), Some("actions/cache/save@v4"));
        let with = steps[0].with.as_ref().unwrap();
        assert_eq!(with["key"], "cargo");
    }

    #[test]
    fn approval_gate_emits_labeled_run_step_and_sets_job_environment() {
        let mut b = WorkflowBuilder::new("deploy");
        let src = b.artifact("src", ArtifactType::SourceTree);
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        let deploy = b.shell_action("deploy", "deploy", &[src], &[], "kubectl apply -f .");
        let csq = b.consequence("ship", ConsequenceKind::Deployment, true);
        b.add_consequence_to(deploy, csq);
        b.actor("runner", &["ubuntu-latest"], &[]);
        let p = plan(&b.build()).unwrap();

        let backend = GithubActionsBackend::default();
        let yaml = backend.emit(&p).unwrap();
        assert!(yaml.contains("Approval gate") || yaml.contains("approval"));
    }

    #[test]
    fn cross_job_artifact_emits_upload_then_download() {
        let mut b = WorkflowBuilder::new("upload-test");
        let src = b.artifact("src", ArtifactType::SourceTree);
        let bin = b.artifact_at("binary", ArtifactType::Binary, "target/release/app");
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        b.shell_action("build", "build", &[src], &[bin], "cargo build --release");
        b.shell_action("test", "test", &[bin], &[], "cargo test");
        b.actor("runner", &["ubuntu-latest"], &[]);
        let p = plan(&b.build()).unwrap();

        let backend = GithubActionsBackend::default();
        let yaml = backend.emit(&p).unwrap();
        assert!(yaml.contains("upload-artifact") || yaml.contains("Upload artifact"));
        assert!(yaml.contains("download-artifact") || yaml.contains("Download artifact"));
    }
}

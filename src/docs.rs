use std::collections::HashSet;

use crate::ir::{ArtifactId, ArtifactType, ConsequenceId, ConsequenceKind, Workflow};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate Markdown documentation from Thread IR.
pub fn generate(workflow: &Workflow) -> String {
    let mut out = String::new();
    write_header(&mut out, workflow);
    write_summary(&mut out, workflow);
    write_inventory(&mut out, workflow);
    write_artifact_graph(&mut out, workflow);
    write_actions(&mut out, workflow);
    write_artifacts(&mut out, workflow);
    write_consequences(&mut out, workflow);
    write_secrets(&mut out, workflow);
    write_release_gates(&mut out, workflow);
    out
}

/// Generate Markdown documentation with an optional backend appendix.
pub fn generate_with_backend(
    workflow: &Workflow,
    backend: &dyn crate::backends::EmittingBackend,
) -> String {
    let mut out = generate(workflow);
    write_backend_summary(&mut out, backend);
    out
}

// ---------------------------------------------------------------------------
// Internal tree structure for the artifact graph
// ---------------------------------------------------------------------------

struct ArtifactNode {
    name: String,
    ty: String,
    children: Vec<ArtifactNode>,
}

fn build_artifact_trees(workflow: &Workflow) -> Vec<ArtifactNode> {
    let root_ids = find_root_artifact_ids(workflow);
    let mut visited = HashSet::new();
    root_ids
        .iter()
        .map(|&id| build_subtree(workflow, id, &mut visited))
        .collect()
}

fn find_root_artifact_ids(workflow: &Workflow) -> Vec<ArtifactId> {
    // "Source" actions: action calls with no artifact inputs.
    // The artifacts they produce are the roots of the data flow.
    let source_outputs: Vec<ArtifactId> = workflow
        .action_calls
        .iter()
        .filter(|a| a.inputs.is_empty())
        .flat_map(|a| a.outputs.iter().copied())
        .collect();

    if !source_outputs.is_empty() {
        return source_outputs;
    }

    // Fallback: externally supplied artifacts (no declared producer).
    let external: Vec<ArtifactId> = workflow
        .artifacts
        .iter()
        .enumerate()
        .filter(|(_, a)| a.producer.is_none())
        .map(|(i, _)| ArtifactId(i as u32))
        .collect();

    if !external.is_empty() {
        return external;
    }

    // Last resort: first artifact in declaration order.
    if !workflow.artifacts.is_empty() {
        vec![ArtifactId(0)]
    } else {
        vec![]
    }
}

fn build_subtree(
    workflow: &Workflow,
    id: ArtifactId,
    visited: &mut HashSet<ArtifactId>,
) -> ArtifactNode {
    visited.insert(id);
    let artifact = workflow.artifact(id);

    let child_ids: Vec<ArtifactId> = workflow
        .action_calls
        .iter()
        .filter(|a| a.inputs.contains(&id))
        .flat_map(|a| a.outputs.iter().copied())
        .filter(|out_id| !visited.contains(out_id))
        .collect();

    let children: Vec<ArtifactNode> = child_ids
        .into_iter()
        .map(|out_id| build_subtree(workflow, out_id, visited))
        .collect();

    ArtifactNode {
        name: artifact.name.to_string(),
        ty: format_artifact_type(&artifact.ty),
        children,
    }
}

fn render_tree(node: &ArtifactNode, prefix: &str, is_last: bool, out: &mut String) {
    let connector = if is_last { "└─ " } else { "├─ " };
    out.push_str(&format!(
        "{prefix}{connector}{} *({})*\n",
        node.name, node.ty
    ));
    let child_prefix = format!("{prefix}{}", if is_last { "   " } else { "│  " });
    for (i, child) in node.children.iter().enumerate() {
        render_tree(child, &child_prefix, i == node.children.len() - 1, out);
    }
}

// ---------------------------------------------------------------------------
// Section writers
// ---------------------------------------------------------------------------

fn write_header(out: &mut String, workflow: &Workflow) {
    out.push_str(&format!("# {}\n\n", workflow.name));
}

fn write_summary(out: &mut String, workflow: &Workflow) {
    out.push_str("## Summary\n\n");

    let action_count = workflow.action_calls.len();
    let artifact_count = workflow.artifacts.len();
    let consequence_count = workflow.consequences.len();

    out.push_str(&format!(
        "{} with **{} action{}**, **{} artifact{}**",
        humanize_name(workflow.name.as_str()),
        action_count,
        if action_count == 1 { "" } else { "s" },
        artifact_count,
        if artifact_count == 1 { "" } else { "s" },
    ));

    if consequence_count > 0 {
        let kinds: Vec<String> = unique_consequence_kinds(workflow);
        out.push_str(&format!(
            ", and **{} consequence{}** ({})",
            consequence_count,
            if consequence_count == 1 { "" } else { "s" },
            kinds.join(", ")
        ));
    }
    out.push_str(".\n\n");

    let verbs = derive_verbs(workflow);
    if !verbs.is_empty() {
        out.push_str(&verbs);
        out.push_str("\n\n");
    }

    if let Some(max) = workflow.policies.max_parallel_jobs {
        out.push_str(&format!("> Policy: maximum {max} parallel jobs.\n\n"));
    }
}

fn write_artifact_graph(out: &mut String, workflow: &Workflow) {
    if workflow.artifacts.is_empty() {
        return;
    }
    out.push_str("## Artifact Graph\n\n");
    out.push_str("```\n");
    let trees = build_artifact_trees(workflow);
    for (i, root) in trees.iter().enumerate() {
        out.push_str(&format!("{} *({})*\n", root.name, root.ty));
        for (j, child) in root.children.iter().enumerate() {
            render_tree(child, "", j == root.children.len() - 1, out);
        }
        if i < trees.len() - 1 {
            out.push('\n');
        }
    }
    out.push_str("```\n\n");
}

fn write_actions(out: &mut String, workflow: &Workflow) {
    if workflow.action_calls.is_empty() {
        return;
    }
    out.push_str("## Actions\n\n");
    out.push_str("| Action | Inputs | Outputs | Consequences |\n");
    out.push_str("|--------|--------|---------|---------------|\n");
    for call in &workflow.action_calls {
        let inputs: Vec<&str> = call
            .inputs
            .iter()
            .map(|&id| workflow.artifact(id).name.as_str())
            .collect();
        let outputs: Vec<&str> = call
            .outputs
            .iter()
            .map(|&id| workflow.artifact(id).name.as_str())
            .collect();
        let csqs: Vec<&str> = call
            .consequences
            .iter()
            .map(|&id| workflow.consequence(id).name.as_str())
            .collect();

        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            call.name,
            if inputs.is_empty() {
                "—".into()
            } else {
                inputs.join(", ")
            },
            if outputs.is_empty() {
                "—".into()
            } else {
                outputs.join(", ")
            },
            if csqs.is_empty() {
                "—".into()
            } else {
                csqs.join(", ")
            },
        ));
    }
    out.push('\n');
}

fn write_artifacts(out: &mut String, workflow: &Workflow) {
    if workflow.artifacts.is_empty() {
        return;
    }
    out.push_str("## Artifacts\n\n");
    out.push_str("| Artifact | Type | Producer | Consumers |\n");
    out.push_str("|----------|------|----------|-----------|\n");
    for (i, artifact) in workflow.artifacts.iter().enumerate() {
        let id = ArtifactId(i as u32);
        let producer = artifact
            .producer
            .map(|pid| format!("`{}`", workflow.action_call(pid).name.as_str()))
            .unwrap_or_else(|| "*(external)*".into());
        let consumers: Vec<String> = workflow
            .action_calls
            .iter()
            .filter(|a| a.inputs.contains(&id))
            .map(|a| format!("`{}`", a.name))
            .collect();
        let path_note = artifact
            .path
            .as_deref()
            .map(|p| format!(" `{p}`"))
            .unwrap_or_default();
        out.push_str(&format!(
            "| `{}`{} | {} | {} | {} |\n",
            artifact.name,
            path_note,
            format_artifact_type(&artifact.ty),
            producer,
            if consumers.is_empty() {
                "*(none)*".into()
            } else {
                consumers.join(", ")
            },
        ));
    }
    out.push('\n');
}

fn write_consequences(out: &mut String, workflow: &Workflow) {
    if workflow.consequences.is_empty() {
        return;
    }
    out.push_str("## Consequences\n\n");
    out.push_str(
        "> Consequences are external mutations or privileged interactions. \
         They are never silently executed and are visible in the plan.\n\n",
    );
    out.push_str("| Consequence | Kind | Triggered By | Approval Required |\n");
    out.push_str("|-------------|------|--------------|-------------------|\n");
    for (i, csq) in workflow.consequences.iter().enumerate() {
        let cid = ConsequenceId(i as u32);
        let triggered_by: Vec<String> = workflow
            .action_calls
            .iter()
            .filter(|a| a.consequences.contains(&cid))
            .map(|a| format!("`{}`", a.name))
            .collect();
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            csq.name,
            format_consequence_kind(&csq.kind),
            if triggered_by.is_empty() {
                "*(none)*".into()
            } else {
                triggered_by.join(", ")
            },
            if csq.requires_approval { "Yes" } else { "No" },
        ));
    }
    out.push('\n');
}

fn write_secrets(out: &mut String, workflow: &Workflow) {
    let secrets: Vec<(&str, Vec<&str>)> = collect_secrets(workflow);
    if secrets.is_empty() {
        return;
    }
    out.push_str("## Secrets\n\n");
    out.push_str("| Secret | Used By |\n");
    out.push_str("|--------|---------|\n");
    for (secret, actions) in &secrets {
        out.push_str(&format!(
            "| `{}` | {} |\n",
            secret,
            actions
                .iter()
                .map(|a| format!("`{a}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push('\n');
}

fn write_release_gates(out: &mut String, workflow: &Workflow) {
    let gated: Vec<_> = workflow
        .consequences
        .iter()
        .enumerate()
        .filter(|(_, c)| c.requires_approval)
        .collect();
    if gated.is_empty() {
        return;
    }

    out.push_str("## Release Gates\n\n");
    out.push_str(
        "The following consequences require explicit approval before they can proceed:\n\n",
    );
    for (i, csq) in &gated {
        let cid = ConsequenceId(*i as u32);
        let triggered_by: Vec<&str> = workflow
            .action_calls
            .iter()
            .filter(|a| a.consequences.contains(&cid))
            .map(|a| a.name.as_str())
            .collect();
        out.push_str(&format!(
            "- **`{}`** ({}) — triggered by: {}\n",
            csq.name,
            format_consequence_kind(&csq.kind),
            if triggered_by.is_empty() {
                "*(none)*".into()
            } else {
                triggered_by
                    .iter()
                    .map(|a| format!("`{a}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ));
    }
    out.push('\n');
}

fn write_inventory(out: &mut String, workflow: &Workflow) {
    let inv = match &workflow.inventory {
        Some(i) => i,
        None => return,
    };
    out.push_str("## Inventory\n\n");
    out.push_str(&format!("**Inventory:** `{}`\n\n", inv.id));

    if !inv.actors.is_empty() {
        out.push_str("**Actors:**\n\n");
        for actor in &inv.actors {
            let caps = actor
                .capabilities
                .iter()
                .map(|c| format!("`{c}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let labels = actor
                .labels
                .iter()
                .map(|l| format!("`{l}`"))
                .collect::<Vec<_>>()
                .join(", ");
            if caps.is_empty() {
                out.push_str(&format!("- `{}` (labels: {})\n", actor.id, labels));
            } else {
                out.push_str(&format!(
                    "- `{}` (labels: {}; capabilities: {})\n",
                    actor.id, labels, caps
                ));
            }
        }
        out.push('\n');
    }

    if !inv.placements.is_empty() {
        out.push_str("**Placement providers:**\n\n");
        for p in &inv.placements {
            let modes = p
                .access_modes
                .iter()
                .map(|m| format!("`{m}`"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("- `{}` (kind: `{}`", p.id, p.kind));
            if !modes.is_empty() {
                out.push_str(&format!("; access: {modes}"));
            }
            out.push_str(")\n");
        }
        out.push('\n');
    }

    if !inv.implementations.is_empty() {
        out.push_str("**Implementations:**\n\n");
        for i in &inv.implementations {
            let mut parts: Vec<String> = vec![];
            if let Some(v) = &i.version {
                parts.push(format!("version: `{v}`"));
            }
            if i.prefer {
                parts.push("preferred".to_string());
            }
            if i.deny {
                parts.push("denied".to_string());
            }
            if parts.is_empty() {
                out.push_str(&format!("- `{}`\n", i.id));
            } else {
                out.push_str(&format!("- `{}` ({})\n", i.id, parts.join("; ")));
            }
        }
        out.push('\n');
    }
}

fn write_backend_summary(out: &mut String, backend: &dyn crate::backends::EmittingBackend) {
    use crate::backends::WorkflowCapabilities as WC;
    out.push_str("## Backend Summary\n\n");
    out.push_str(&format!("**Backend:** `{}`\n\n", backend.id()));

    let wc = backend.workflow_capabilities();
    let mut workflow_features: Vec<&str> = vec![];
    if wc.contains(WC::JOBS) {
        workflow_features.push("jobs");
    }
    if wc.contains(WC::DEPENDENCIES) {
        workflow_features.push("dependencies");
    }
    if wc.contains(WC::CONDITIONS) {
        workflow_features.push("conditions");
    }
    if wc.contains(WC::MATRICES) {
        workflow_features.push("matrices");
    }
    if wc.contains(WC::PERMISSIONS) {
        workflow_features.push("permissions");
    }
    if wc.contains(WC::SECRETS) {
        workflow_features.push("secrets");
    }
    if wc.contains(WC::APPROVALS) {
        workflow_features.push("approvals");
    }
    if wc.contains(WC::RUNNER_SELECTION) {
        workflow_features.push("runner selection");
    }
    if !workflow_features.is_empty() {
        out.push_str(&format!(
            "**Workflow features:** {}\n\n",
            workflow_features.join(", ")
        ));
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn format_artifact_type(ty: &ArtifactType) -> String {
    match ty {
        ArtifactType::SourceTree => "SourceTree".into(),
        ArtifactType::Wheel => "Wheel".into(),
        ArtifactType::Binary => "Binary".into(),
        ArtifactType::ContainerImage => "ContainerImage".into(),
        ArtifactType::Sbom => "SBOM".into(),
        ArtifactType::Signature => "Signature".into(),
        ArtifactType::ReleaseBundle => "ReleaseBundle".into(),
        ArtifactType::TestReport => "TestReport".into(),
        ArtifactType::CoverageData => "CoverageData".into(),
        ArtifactType::DocsSite => "DocsSite".into(),
        ArtifactType::ProfileData => "ProfileData".into(),
        ArtifactType::Model => "Model".into(),
        ArtifactType::Custom(s) => s.clone(),
    }
}

fn format_consequence_kind(kind: &ConsequenceKind) -> &'static str {
    match kind {
        ConsequenceKind::Network => "Network",
        ConsequenceKind::SecretAccess => "SecretAccess",
        ConsequenceKind::GitWrite => "GitWrite",
        ConsequenceKind::PublishRelease => "PublishRelease",
        ConsequenceKind::Deployment => "Deployment",
        ConsequenceKind::CommentOnPr => "CommentOnPr",
        ConsequenceKind::Custom(_) => "Custom",
    }
}

fn humanize_name(name: &str) -> String {
    let words: Vec<String> = name
        .split(['_', '-'])
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect();
    words.join(" ")
}

fn derive_verbs(workflow: &Workflow) -> String {
    let mut verbs: Vec<&str> = vec![];
    let has_test = workflow
        .action_calls
        .iter()
        .any(|a| a.name.as_str().contains("test"));
    let has_build = workflow
        .action_calls
        .iter()
        .any(|a| a.name.as_str().contains("build") || a.name.as_str().contains("compile"));

    if has_build {
        verbs.push("builds artifacts");
    }
    if has_test {
        verbs.push("runs tests");
    }

    for csq in &workflow.consequences {
        match csq.kind {
            ConsequenceKind::PublishRelease => verbs.push("publishes releases"),
            ConsequenceKind::Deployment => verbs.push("deploys"),
            ConsequenceKind::SecretAccess => {}
            ConsequenceKind::GitWrite => verbs.push("writes to the repository"),
            ConsequenceKind::CommentOnPr => verbs.push("comments on pull requests"),
            _ => {}
        }
    }

    let verbs: Vec<&str> = {
        let mut seen = HashSet::new();
        verbs.into_iter().filter(|v| seen.insert(*v)).collect()
    };

    match verbs.len() {
        0 => String::new(),
        1 => format!("This workflow {}.", verbs[0]),
        2 => format!("This workflow {} and {}.", verbs[0], verbs[1]),
        _ => {
            let (last, rest) = verbs.split_last().unwrap();
            format!("This workflow {}, and {}.", rest.join(", "), last)
        }
    }
}

fn unique_consequence_kinds(workflow: &Workflow) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    workflow
        .consequences
        .iter()
        .map(|c| format_consequence_kind(&c.kind).to_string())
        .filter(|k| seen.insert(k.clone()))
        .collect()
}

fn collect_secrets(workflow: &Workflow) -> Vec<(&str, Vec<&str>)> {
    let mut map: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for call in &workflow.action_calls {
        for secret in &call.secrets {
            map.entry(secret.as_str())
                .or_default()
                .push(call.name.as_str());
        }
    }
    map.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{ArtifactType, ConsequenceKind, WorkflowBuilder};

    fn build_workflow() -> Workflow {
        let mut b = WorkflowBuilder::new("ci");
        let src = b.artifact("src", ArtifactType::SourceTree);
        let bin = b.artifact_at("binary", ArtifactType::Binary, "target/release/app");
        let report = b.artifact_at("report", ArtifactType::TestReport, "test-results.xml");
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        b.shell_action("build", "build", &[src], &[bin], "cargo build --release");
        b.shell_action("test", "test", &[bin], &[report], "cargo test");
        b.actor("runner", &["ubuntu-latest"], &[]);
        b.build()
    }

    fn build_release_workflow() -> Workflow {
        let mut b = WorkflowBuilder::new("release");
        let src = b.artifact("src", ArtifactType::SourceTree);
        let bin = b.artifact_at("binary", ArtifactType::Binary, "target/release/app");
        let report = b.artifact_at("report", ArtifactType::TestReport, "test-results.xml");
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        b.shell_action("build", "build", &[src], &[bin], "cargo build --release");
        let test_action = b.shell_action("test", "test", &[bin], &[report], "cargo test");
        let deploy_action =
            b.shell_action("deploy", "deploy", &[report], &[], "kubectl apply -f .");
        let publish_csq = b.consequence("publish", ConsequenceKind::PublishRelease, true);
        let deploy_csq = b.consequence("deploy", ConsequenceKind::Deployment, false);
        b.add_consequence_to(deploy_action, publish_csq);
        b.add_consequence_to(deploy_action, deploy_csq);

        let mut wf = b.build();
        wf.action_calls[test_action.idx()]
            .secrets
            .push("SIGNING_KEY".into());
        wf
    }

    #[test]
    fn generates_header_with_workflow_name() {
        let wf = build_workflow();
        let doc = generate(&wf);
        assert!(doc.starts_with("# ci\n"));
    }

    #[test]
    fn summary_section_has_action_and_artifact_counts() {
        let wf = build_workflow();
        let doc = generate(&wf);
        assert!(doc.contains("**3 actions**"));
        assert!(doc.contains("**3 artifacts**"));
    }

    #[test]
    fn artifact_graph_section_present() {
        let wf = build_workflow();
        let doc = generate(&wf);
        assert!(doc.contains("## Artifact Graph"));
        assert!(doc.contains("src"));
        assert!(doc.contains("binary"));
        assert!(doc.contains("report"));
    }

    #[test]
    fn artifact_graph_shows_root_first() {
        let wf = build_workflow();
        let doc = generate(&wf);
        let graph_start = doc.find("## Artifact Graph").unwrap();
        let src_pos = doc[graph_start..].find("src").unwrap();
        let binary_pos = doc[graph_start..].find("binary").unwrap();
        assert!(src_pos < binary_pos, "root should appear before children");
    }

    #[test]
    fn actions_table_present() {
        let wf = build_workflow();
        let doc = generate(&wf);
        assert!(doc.contains("## Actions"));
        assert!(doc.contains("`checkout`"));
        assert!(doc.contains("`build`"));
        assert!(doc.contains("`test`"));
    }

    #[test]
    fn actions_table_shows_io() {
        let wf = build_workflow();
        let doc = generate(&wf);
        let actions_start = doc.find("## Actions").unwrap();
        let section = &doc[actions_start..];
        assert!(section.contains("src"));
        assert!(section.contains("binary"));
    }

    #[test]
    fn artifacts_table_present() {
        let wf = build_workflow();
        let doc = generate(&wf);
        assert!(doc.contains("## Artifacts"));
        assert!(doc.contains("Binary"));
        assert!(doc.contains("TestReport"));
    }

    #[test]
    fn artifacts_table_shows_path() {
        let wf = build_workflow();
        let doc = generate(&wf);
        assert!(doc.contains("target/release/app"));
        assert!(doc.contains("test-results.xml"));
    }

    #[test]
    fn artifacts_table_shows_producer_and_consumers() {
        let wf = build_workflow();
        let doc = generate(&wf);
        let art_start = doc.find("## Artifacts").unwrap();
        let section = &doc[art_start..];
        assert!(section.contains("`build`"));
        assert!(section.contains("`test`"));
    }

    #[test]
    fn consequences_section_present_when_non_empty() {
        let wf = build_release_workflow();
        let doc = generate(&wf);
        assert!(doc.contains("## Consequences"));
        assert!(doc.contains("PublishRelease"));
        assert!(doc.contains("Deployment"));
    }

    #[test]
    fn consequences_section_absent_when_empty() {
        let wf = build_workflow();
        let doc = generate(&wf);
        assert!(!doc.contains("## Consequences"));
    }

    #[test]
    fn release_gates_section_present_for_approval_required() {
        let wf = build_release_workflow();
        let doc = generate(&wf);
        assert!(doc.contains("## Release Gates"));
        assert!(doc.contains("publish"));
    }

    #[test]
    fn release_gates_absent_when_no_approval() {
        let wf = build_workflow();
        let doc = generate(&wf);
        assert!(!doc.contains("## Release Gates"));
    }

    #[test]
    fn secrets_section_present_when_non_empty() {
        let wf = build_release_workflow();
        let doc = generate(&wf);
        assert!(doc.contains("## Secrets"));
        assert!(doc.contains("SIGNING_KEY"));
        assert!(doc.contains("`test`"));
    }

    #[test]
    fn secrets_section_absent_when_empty() {
        let wf = build_workflow();
        let doc = generate(&wf);
        assert!(!doc.contains("## Secrets"));
    }

    #[test]
    fn generation_is_deterministic() {
        let wf = build_release_workflow();
        let a = generate(&wf);
        let b = generate(&wf);
        assert_eq!(a, b);
    }

    #[test]
    fn humanize_name_snake_case() {
        assert_eq!(humanize_name("build_wheel"), "Build Wheel");
        assert_eq!(humanize_name("release"), "Release");
        assert_eq!(humanize_name("test_and_deploy"), "Test And Deploy");
    }

    #[test]
    fn backend_summary_included_when_backend_provided() {
        use crate::backends::github::GithubActionsBackend;
        let wf = build_workflow();
        let backend = GithubActionsBackend::default();
        let doc = generate_with_backend(&wf, &backend);
        assert!(doc.contains("## Backend Summary"));
        assert!(doc.contains("`github`"));
        assert!(doc.contains("approvals"));
    }

    #[test]
    fn backend_summary_shows_workflow_capabilities() {
        use crate::backends::local::LocalBackend;
        let wf = build_workflow();
        let doc = generate_with_backend(&wf, &LocalBackend::podman());
        assert!(doc.contains("## Backend Summary"));
        assert!(doc.contains("approvals"));
    }

    #[test]
    fn inventory_section_absent_without_inventory() {
        let wf = build_workflow_no_inventory();
        let doc = generate(&wf);
        assert!(!doc.contains("## Inventory"));
    }

    #[test]
    fn inventory_section_present_when_declared() {
        let wf = build_workflow();
        let doc = generate(&wf);
        assert!(doc.contains("## Inventory"));
        assert!(doc.contains("ubuntu-latest"));
        assert!(doc.contains("runner"));
    }

    #[test]
    fn inventory_section_shows_placement_providers() {
        let mut b = WorkflowBuilder::new("ci");
        let src = b.artifact("src", ArtifactType::SourceTree);
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        b.actor("runner", &["ubuntu-latest"], &[]);
        b.inventory_placement(
            "workspace",
            "volume",
            &["mount_rw", "same_host"],
            &["runner"],
        );
        let wf = b.build();
        let doc = generate(&wf);
        assert!(doc.contains("workspace"));
        assert!(doc.contains("`mount_rw`"));
        assert!(doc.contains("`same_host`"));
    }

    #[test]
    fn inventory_section_shows_implementations() {
        let mut b = WorkflowBuilder::new("ci");
        let src = b.artifact("src", ArtifactType::SourceTree);
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        b.actor("runner", &["ubuntu-latest"], &[]);
        b.implementation("git", None, false, false);
        b.implementation("maturin", Some("1.7"), true, false);
        b.implementation("docker", None, false, true);
        let wf = b.build();
        let doc = generate(&wf);
        assert!(doc.contains("**Implementations:**"));
        assert!(doc.contains("`git`"));
        assert!(doc.contains("`maturin`"));
        assert!(doc.contains("version: `1.7`"));
        assert!(doc.contains("preferred"));
        assert!(doc.contains("`docker`"));
        assert!(doc.contains("denied"));
    }

    fn build_workflow_no_inventory() -> Workflow {
        let mut b = WorkflowBuilder::new("ci");
        let src = b.artifact("src", ArtifactType::SourceTree);
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        b.build()
    }
}

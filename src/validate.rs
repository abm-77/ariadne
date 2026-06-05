use crate::diagnostics::{DiagCode, Diagnostic};
use std::collections::HashSet;
use crate::ir::{ActionId, ArtifactId, Workflow};

pub fn validate(workflow: &Workflow) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    let n_artifacts = workflow.artifacts.len();
    let n_actions = workflow.actions.len();
    let n_effects = workflow.effects.len();

    // Bounds-check
    for action in &workflow.actions {
        for &id in &action.inputs {
            if id.idx() >= n_artifacts {
                diags.push(Diagnostic::error(
                    DiagCode::IndexOutOfBounds,
                    format!("Action '{}' references artifact index {} but only {} artifacts exist",
                        action.name, id.0, n_artifacts),
                ));
            }
        }
        for &id in &action.outputs {
            if id.idx() >= n_artifacts {
                diags.push(Diagnostic::error(
                    DiagCode::IndexOutOfBounds,
                    format!("Action '{}' declares output artifact index {} but only {} artifacts exist",
                        action.name, id.0, n_artifacts),
                ));
            }
        }
        for &id in &action.effects {
            if id.idx() >= n_effects {
                diags.push(Diagnostic::error(
                    DiagCode::UnknownEffect,
                    format!("Action '{}' references effect index {} but only {} effects exist",
                        action.name, id.0, n_effects),
                ));
            }
        }
    }

    for artifact in &workflow.artifacts {
        if let Some(producer) = artifact.producer
            && producer.idx() >= n_actions {
                diags.push(Diagnostic::error(
                    DiagCode::IndexOutOfBounds,
                    format!("Artifact '{}' declares producer index {} but only {} actions exist",
                        artifact.name, producer.0, n_actions),
                ));
            }
    }

    // Stop early if there are index errors — further checks would panic.
    if diags.iter().any(|d| d.is_error()) {
        return diags;
    }

    // Producer consistency: each artifact's declared producer must list that artifact as an output.
    for (idx, artifact) in workflow.artifacts.iter().enumerate() {
        let art_id = ArtifactId(idx as u32);
        if let Some(producer_id) = artifact.producer {
            let producer = workflow.action(producer_id);
            if !producer.outputs.contains(&art_id) {
                diags.push(Diagnostic::error(
                    DiagCode::MissingProducer,
                    format!("Artifact '{}' declares producer '{}' but that action does not list it as an output",
                        artifact.name, producer.name),
                ));
            }
        }
    }

    // Each action's output must have its producer field pointing back to this action.
    for (idx, action) in workflow.actions.iter().enumerate() {
        let action_id = ActionId(idx as u32);
        for &out_id in &action.outputs {
            match workflow.artifact(out_id).producer {
                Some(p) if p == action_id => {}
                Some(p) => diags.push(Diagnostic::error(
                    DiagCode::MissingProducer,
                    format!("Action '{}' lists artifact '{}' as output, but that artifact's producer is '{}'",
                        action.name,
                        workflow.artifact(out_id).name,
                        workflow.action(p).name),
                )),
                None => diags.push(Diagnostic::error(
                    DiagCode::MissingProducer,
                    format!("Action '{}' lists artifact '{}' as output, but that artifact has no producer declared",
                        action.name, workflow.artifact(out_id).name),
                )),
            }
        }
    }

    // Unused inputs
    let consumed: HashSet<ArtifactId> = workflow.actions.iter()
        .flat_map(|a| a.inputs.iter().copied())
        .collect();

    for (idx, artifact) in workflow.artifacts.iter().enumerate() {
        let art_id = ArtifactId(idx as u32);
        if artifact.producer.is_none() && !consumed.contains(&art_id) {
            diags.push(
                Diagnostic::warning(DiagCode::UnusedInput,
                    format!("Artifact '{}' has no producer (external input) and is not consumed by any action",
                        artifact.name))
                .with_fix("remove the artifact declaration or add an action that consumes it"),
            );
        }
    }

    // Cycle detection
    if let Some(msg) = detect_cycle(workflow) {
        diags.push(Diagnostic::error(DiagCode::CycleDetected, msg));
    }

    // Op call validation: when an action's op has a definition, check that its
    // input/output artifact types match the declared ports.
    validate_op_calls(workflow, &mut diags);

    diags
}

fn validate_op_calls(workflow: &Workflow, diags: &mut Vec<Diagnostic>) {
    use crate::ir::PortKind;

    for action in &workflow.actions {
        let Some(def) = workflow.find_op(action.op.as_str()) else { continue };

        let art_inputs: Vec<_> = def.inputs.iter().filter(|p| p.kind == PortKind::Artifact).collect();
        let art_outputs: Vec<_> = def.outputs.iter().filter(|p| p.kind == PortKind::Artifact).collect();

        if action.inputs.len() != art_inputs.len() {
            diags.push(Diagnostic::error(
                DiagCode::OpPortMismatch,
                format!(
                    "Action '{}' calls op '{}' with {} artifact input(s) but op declares {}",
                    action.name, def.id, action.inputs.len(), art_inputs.len()
                ),
            ));
        }

        if action.outputs.len() != art_outputs.len() {
            diags.push(Diagnostic::error(
                DiagCode::OpPortMismatch,
                format!(
                    "Action '{}' calls op '{}' with {} artifact output(s) but op declares {}",
                    action.name, def.id, action.outputs.len(), art_outputs.len()
                ),
            ));
        }

        // Type-check inputs positionally.
        for (port, &art_id) in art_inputs.iter().zip(action.inputs.iter()) {
            let artifact = workflow.artifact(art_id);
            let actual = artifact_type_str(&artifact.ty);
            if actual != port.ty {
                diags.push(Diagnostic::error(
                    DiagCode::TypeMismatch,
                    format!(
                        "Action '{}' passes artifact '{}' (type {}) to port '{}' of op '{}' which expects {}",
                        action.name, artifact.name, actual, port.name, def.id, port.ty
                    ),
                ));
            }
        }

        // Type-check outputs positionally.
        for (port, &art_id) in art_outputs.iter().zip(action.outputs.iter()) {
            let artifact = workflow.artifact(art_id);
            let actual = artifact_type_str(&artifact.ty);
            if actual != port.ty {
                diags.push(Diagnostic::error(
                    DiagCode::TypeMismatch,
                    format!(
                        "Action '{}' produces artifact '{}' (type {}) for port '{}' of op '{}' which declares {}",
                        action.name, artifact.name, actual, port.name, def.id, port.ty
                    ),
                ));
            }
        }
    }
}

fn artifact_type_str(ty: &crate::ir::ArtifactType) -> String {
    use crate::ir::ArtifactType;
    match ty {
        ArtifactType::SourceTree => "SourceTree".into(),
        ArtifactType::Wheel => "Wheel".into(),
        ArtifactType::Binary => "Binary".into(),
        ArtifactType::ContainerImage => "ContainerImage".into(),
        ArtifactType::Sbom => "Sbom".into(),
        ArtifactType::Signature => "Signature".into(),
        ArtifactType::ReleaseBundle => "ReleaseBundle".into(),
        ArtifactType::TestReport => "TestReport".into(),
        ArtifactType::Model => "Model".into(),
        ArtifactType::Custom(s) => s.clone(),
    }
}

fn detect_cycle(workflow: &Workflow) -> Option<String> {
    // deps[action_idx] = indices of actions this action depends on
    let deps: Vec<Vec<usize>> = workflow.actions.iter().map(|action| {
        action.inputs.iter()
            .filter_map(|&art_id| workflow.artifact(art_id).producer)
            .filter(|&pred| pred.idx() != workflow.actions.iter().position(|a| a.name == action.name).unwrap_or(usize::MAX))
            .map(|id| id.idx())
            .collect()
    }).collect();

    let n = workflow.actions.len();
    let mut visited = vec![false; n];
    let mut in_stack = vec![false; n];

    for start in 0..n {
        if !visited[start] && dfs_has_cycle(start, &deps, &mut visited, &mut in_stack) {
            return Some("Cycle detected in action dependency graph".into());
        }
    }
    None
}

fn dfs_has_cycle(
    node: usize,
    deps: &[Vec<usize>],
    visited: &mut Vec<bool>,
    in_stack: &mut Vec<bool>,
) -> bool {
    visited[node] = true;
    in_stack[node] = true;

    for &neighbor in &deps[node] {
        if !visited[neighbor] {
            if dfs_has_cycle(neighbor, deps, visited, in_stack) {
                return true;
            }
        } else if in_stack[neighbor] {
            return true;
        }
    }

    in_stack[node] = false;
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Severity;
    use crate::ir::*;

    fn checkout_build_test() -> Workflow {
        let mut b = WorkflowBuilder::new("test");
        let src = b.artifact("source", ArtifactType::SourceTree);
        let bin = b.artifact("binary", ArtifactType::Binary);
        let rep = b.artifact("test-report", ArtifactType::TestReport);
        b.action("checkout", "checkout", &[], &[src]);
        b.action("build", "build", &[src], &[bin]);
        b.action("test", "test", &[bin], &[rep]);
        b.build()
    }

    #[test]
    fn valid_workflow_has_no_errors() {
        let diags = validate(&checkout_build_test());
        assert!(
            !diags.iter().any(|d| d.is_error()),
            "unexpected errors: {diags:?}"
        );
    }

    #[test]
    fn producer_pointing_to_nonexistent_action_is_error() {
        let mut wf = checkout_build_test();
        // Force artifact 0 to point at an out-of-bounds action index
        wf.artifacts[0].producer = Some(ActionId(99));
        let diags = validate(&wf);
        assert!(diags.iter().any(|d| d.code == DiagCode::IndexOutOfBounds));
    }

    #[test]
    fn output_not_matching_producer_field_is_error() {
        let mut wf = checkout_build_test();
        // artifact 0 (source) correctly points at action 0 (checkout),
        // but now remove source from checkout's outputs → mismatch
        wf.actions[0].outputs.clear();
        let diags = validate(&wf);
        assert!(diags.iter().any(|d| d.code == DiagCode::MissingProducer));
    }

    #[test]
    fn cycle_is_detected() {
        let mut b = WorkflowBuilder::new("cyclic");
        let x = b.artifact("x", ArtifactType::Binary);
        let y = b.artifact("y", ArtifactType::Binary);
        // action-a outputs x, takes y as input
        // action-b outputs y, takes x as input → cycle
        b.action("a", "a", &[y], &[x]);
        b.action("b", "b", &[x], &[y]);
        let diags = validate(&b.build());
        assert!(diags.iter().any(|d| d.code == DiagCode::CycleDetected));
    }

    #[test]
    fn unused_external_input_triggers_warning() {
        let mut b = WorkflowBuilder::new("unused");
        let _dead = b.artifact("dead-input", ArtifactType::Binary); // no producer, never used
        let src = b.artifact("source", ArtifactType::SourceTree);
        b.action("checkout", "checkout", &[], &[src]);
        let diags = validate(&b.build());
        assert!(
            diags.iter().any(|d| d.code == DiagCode::UnusedInput && d.severity == Severity::Warning)
        );
    }
}

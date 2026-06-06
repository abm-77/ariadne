use crate::diagnostics::{DiagCode, Diagnostic};
use crate::ir::{ActionCallId, ArtifactId, Workflow};
use std::collections::HashSet;

pub fn validate(workflow: &Workflow) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    let n_artifacts = workflow.artifacts.len();
    let n_actions = workflow.action_calls.len();
    let n_effects = workflow.consequences.len();

    // Bounds-check
    for action in &workflow.action_calls {
        for &id in &action.inputs {
            if id.idx() >= n_artifacts {
                diags.push(Diagnostic::error(
                    DiagCode::IndexOutOfBounds,
                    format!(
                        "ActionCall '{}' references artifact index {} but only {} artifacts exist",
                        action.name, id.0, n_artifacts
                    ),
                ));
            }
        }
        for &id in &action.outputs {
            if id.idx() >= n_artifacts {
                diags.push(Diagnostic::error(
                    DiagCode::IndexOutOfBounds,
                    format!("ActionCall '{}' declares output artifact index {} but only {} artifacts exist",
                        action.name, id.0, n_artifacts),
                ));
            }
        }
        for &id in &action.consequences {
            if id.idx() >= n_effects {
                diags.push(Diagnostic::error(
                    DiagCode::UnknownConsequence,
                    format!(
                        "ActionCall '{}' references effect index {} but only {} effects exist",
                        action.name, id.0, n_effects
                    ),
                ));
            }
        }
    }

    for artifact in &workflow.artifacts {
        if let Some(producer) = artifact.producer
            && producer.idx() >= n_actions
        {
            diags.push(Diagnostic::error(
                DiagCode::IndexOutOfBounds,
                format!(
                    "Artifact '{}' declares producer index {} but only {} actions exist",
                    artifact.name, producer.0, n_actions
                ),
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
            let producer = workflow.action_call(producer_id);
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
    for (idx, action) in workflow.action_calls.iter().enumerate() {
        let action_id = ActionCallId(idx as u32);
        for &out_id in &action.outputs {
            match workflow.artifact(out_id).producer {
                Some(p) if p == action_id => {}
                Some(p) => diags.push(Diagnostic::error(
                    DiagCode::MissingProducer,
                    format!("ActionCall '{}' lists artifact '{}' as output, but that artifact's producer is '{}'",
                        action.name,
                        workflow.artifact(out_id).name,
                        workflow.action_call(p).name),
                )),
                None => diags.push(Diagnostic::error(
                    DiagCode::MissingProducer,
                    format!("ActionCall '{}' lists artifact '{}' as output, but that artifact has no producer declared",
                        action.name, workflow.artifact(out_id).name),
                )),
            }
        }
    }

    // Unused inputs
    let consumed: HashSet<ArtifactId> = workflow
        .action_calls
        .iter()
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
    validate_action_def_calls(workflow, &mut diags);

    // Resource feasibility: an action declaring resource requirements must have
    // at least one actor in the inventory that can satisfy them.
    for action in &workflow.action_calls {
        if let Some(req) = action.resources.as_ref().filter(|r| !r.is_empty())
            && !workflow.actors().iter().any(|a| a.satisfies(req))
        {
            diags.push(Diagnostic::error(
                DiagCode::UnsatisfiableResources,
                format!(
                    "action '{}' requires resources no actor in the inventory satisfies",
                    action.name
                ),
            ));
        }
    }

    diags
}

fn validate_action_def_calls(workflow: &Workflow, diags: &mut Vec<Diagnostic>) {
    use crate::ir::PortKind;

    for action in &workflow.action_calls {
        let Some(def) = workflow.find_action_def(action.action.as_str()) else {
            continue;
        };

        let art_inputs: Vec<_> = def
            .inputs
            .iter()
            .filter(|p| p.kind == PortKind::Artifact)
            .collect();
        let art_outputs: Vec<_> = def
            .outputs
            .iter()
            .filter(|p| p.kind == PortKind::Artifact)
            .collect();

        if action.inputs.len() != art_inputs.len() {
            diags.push(Diagnostic::error(
                DiagCode::ActionPortMismatch,
                format!(
                    "ActionCall '{}' calls op '{}' with {} artifact input(s) but op declares {}",
                    action.name,
                    def.id,
                    action.inputs.len(),
                    art_inputs.len()
                ),
            ));
        }

        if action.outputs.len() != art_outputs.len() {
            diags.push(Diagnostic::error(
                DiagCode::ActionPortMismatch,
                format!(
                    "ActionCall '{}' calls op '{}' with {} artifact output(s) but op declares {}",
                    action.name,
                    def.id,
                    action.outputs.len(),
                    art_outputs.len()
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
                        "ActionCall '{}' passes artifact '{}' (type {}) to port '{}' of op '{}' which expects {}",
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
                        "ActionCall '{}' produces artifact '{}' (type {}) for port '{}' of op '{}' which declares {}",
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
        ArtifactType::CoverageData => "CoverageData".into(),
        ArtifactType::DocsSite => "DocsSite".into(),
        ArtifactType::ProfileData => "ProfileData".into(),
        ArtifactType::Model => "Model".into(),
        ArtifactType::Custom(s) => s.clone(),
    }
}

fn detect_cycle(workflow: &Workflow) -> Option<String> {
    // deps[action_idx] = indices of actions this action depends on
    let deps: Vec<Vec<usize>> = workflow
        .action_calls
        .iter()
        .map(|action| {
            action
                .inputs
                .iter()
                .filter_map(|&art_id| workflow.artifact(art_id).producer)
                .filter(|&pred| {
                    pred.idx()
                        != workflow
                            .action_calls
                            .iter()
                            .position(|a| a.name == action.name)
                            .unwrap_or(usize::MAX)
                })
                .map(|id| id.idx())
                .collect()
        })
        .collect();

    let n = workflow.action_calls.len();
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

    #[test]
    fn unsatisfiable_resources_is_an_error() {
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::SourceTree);
        let a = b.shell_action("heavy", "heavy", &[], &[src], "make");
        b.actor("small", &["ubuntu-latest"], &[]);
        let mut wf = b.build();
        // Actor advertises nothing; action needs 8 CPU.
        wf.action_calls[a.idx()].resources = Some(Resources {
            cpu: Some(8),
            ..Default::default()
        });
        let diags = validate(&wf);
        assert!(
            diags
                .iter()
                .any(|d| d.code == DiagCode::UnsatisfiableResources)
        );
    }

    #[test]
    fn satisfiable_resources_validates_clean() {
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::SourceTree);
        let a = b.shell_action("heavy", "heavy", &[], &[src], "make");
        b.actor("big", &["ubuntu-large"], &[]);
        let mut wf = b.build();
        wf.inventory.as_mut().unwrap().actors[0].resources = Some(Resources {
            cpu: Some(16),
            memory: Some("64Gi".into()),
            ..Default::default()
        });
        wf.action_calls[a.idx()].resources = Some(Resources {
            cpu: Some(8),
            memory: Some("32Gi".into()),
            ..Default::default()
        });
        assert!(
            !validate(&wf)
                .iter()
                .any(|d| d.code == DiagCode::UnsatisfiableResources)
        );
    }

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
        wf.artifacts[0].producer = Some(ActionCallId(99));
        let diags = validate(&wf);
        assert!(diags.iter().any(|d| d.code == DiagCode::IndexOutOfBounds));
    }

    #[test]
    fn output_not_matching_producer_field_is_error() {
        let mut wf = checkout_build_test();
        // artifact 0 (source) correctly points at action 0 (checkout),
        // but now remove source from checkout's outputs → mismatch
        wf.action_calls[0].outputs.clear();
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
            diags
                .iter()
                .any(|d| d.code == DiagCode::UnusedInput && d.severity == Severity::Warning)
        );
    }
}

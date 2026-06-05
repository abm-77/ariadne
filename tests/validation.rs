use ariadne::diagnostics::{DiagCode, Severity};
use ariadne::ir::*;
use ariadne::validate::validate;

fn fixture() -> Workflow {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/simple-build-test.tir.json");
    ariadne::proto::load(&path).expect("fixture load failed")
}

fn simple_workflow() -> Workflow {
    let mut b = WorkflowBuilder::new("Simple Build and Test");
    let src = b.artifact("source", ArtifactType::SourceTree);
    let bin = b.artifact("binary", ArtifactType::Binary);
    let rep = b.artifact("test-report", ArtifactType::TestReport);
    b.action("checkout", "checkout", &[], &[src]);
    b.action("build", "build", &[src], &[bin]);
    b.action("test", "test", &[bin], &[rep]);
    b.build()
}

fn cyclic_workflow() -> Workflow {
    let mut b = WorkflowBuilder::new("Cyclic");
    let x = b.artifact("x", ArtifactType::Binary);
    let y = b.artifact("y", ArtifactType::Binary);
    b.action("a", "a", &[y], &[x]);
    b.action("b", "b", &[x], &[y]);
    b.build()
}

#[test]
fn fixture_parses_and_validates_clean() {
    let diags = validate(&fixture());
    let errors: Vec<_> = diags.iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "fixture errors: {errors:?}");
}

#[test]
fn simple_workflow_validates_clean() {
    let diags = validate(&simple_workflow());
    assert!(!diags.iter().any(|d| d.is_error()), "{diags:?}");
}

#[test]
fn missing_producer_action_is_error() {
    let mut wf = simple_workflow();
    // artifact 0 (source) says producer is action 0 (checkout),
    // but remove checkout from the actions list so the index is out of bounds
    wf.actions.clear();
    let diags = validate(&wf);
    assert!(diags.iter().any(|d| d.code == DiagCode::IndexOutOfBounds || d.code == DiagCode::MissingProducer));
}

#[test]
fn output_not_listed_by_action_is_error() {
    let mut wf = simple_workflow();
    // Clear checkout's outputs while artifact 0 still says producer=0
    wf.actions[0].outputs.clear();
    let diags = validate(&wf);
    assert!(diags.iter().any(|d| d.code == DiagCode::MissingProducer), "{diags:?}");
}

#[test]
fn out_of_bounds_artifact_ref_is_error() {
    let mut wf = simple_workflow();
    // Give build action an input index that doesn't exist
    wf.actions[1].inputs.push(ArtifactId(99));
    let diags = validate(&wf);
    assert!(diags.iter().any(|d| d.code == DiagCode::IndexOutOfBounds));
}

#[test]
fn out_of_bounds_effect_ref_is_error() {
    let mut wf = simple_workflow();
    wf.actions[0].effects.push(EffectId(99));
    let diags = validate(&wf);
    assert!(diags.iter().any(|d| d.code == DiagCode::UnknownEffect));
}

#[test]
fn cycle_is_detected() {
    let diags = validate(&cyclic_workflow());
    assert!(diags.iter().any(|d| d.code == DiagCode::CycleDetected), "{diags:?}");
}

#[test]
fn unused_external_input_is_a_warning() {
    let mut b = WorkflowBuilder::new("t");
    let _orphan = b.artifact("orphan", ArtifactType::Binary); // no producer, never consumed
    let src = b.artifact("src", ArtifactType::SourceTree);
    b.action("checkout", "checkout", &[], &[src]);
    let diags = validate(&b.build());
    assert!(diags.iter().any(|d| d.code == DiagCode::UnusedInput && d.severity == Severity::Warning));
}

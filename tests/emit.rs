use ariadne::backends::github::{emit, EmitOptions, GithubActionsBackend};
use ariadne::backends::Backend;
use ariadne::ir::Workflow;

fn fixture() -> Workflow {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/simple-build-test.tir.json");
    ariadne::proto::load(&path).expect("fixture load failed")
}

fn golden() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/simple-build-test.yaml");
    std::fs::read_to_string(path).expect("golden missing")
}

#[test]
fn golden_yaml_matches() {
    let plan = ariadne::planner::plan(&fixture()).expect("plan failed");
    assert_eq!(emit(&plan, &EmitOptions::default()), golden());
}

#[test]
fn build_needs_checkout() {
    let plan = ariadne::planner::plan(&fixture()).unwrap();
    let build = plan.units.iter().find(|u| u.action_name == "build").unwrap();
    assert!(build.needs.iter().any(|n| n == "checkout"), "{:?}", build.needs);
}

#[test]
fn test_needs_build() {
    let plan = ariadne::planner::plan(&fixture()).unwrap();
    let test = plan.units.iter().find(|u| u.action_name == "test").unwrap();
    assert!(test.needs.iter().any(|n| n == "build"), "{:?}", test.needs);
}

#[test]
fn emitted_yaml_contains_expected_keys() {
    let plan = ariadne::planner::plan(&fixture()).unwrap();
    let yaml = emit(&plan, &EmitOptions::default());
    assert!(yaml.contains("name: Simple Build and Test"));
    assert!(yaml.contains("runs-on: ubuntu-latest"));
    assert!(yaml.contains("actions/upload-artifact@v4"));
    assert!(yaml.contains("actions/download-artifact@v4"));
}

#[test]
fn copy_fallback_emits_without_warnings() {
    // No shared placement is declared, so the always-legal copy baseline is
    // correct and carries no "fallback" warning — that warning is reserved for
    // the optimizer downgrading a declared mount it couldn't realize.
    let wf = fixture();
    assert!(wf.actors().iter().all(|a| !a.capabilities.iter().any(|c| c == "mount")));
    let plan = ariadne::planner::plan(&wf).unwrap();
    assert!(plan.diagnostics.iter()
        .all(|d| d.code != ariadne::diagnostics::DiagCode::FallbackPlacementSelected));
    let yaml = emit(&plan, &EmitOptions::default());
    assert!(yaml.contains("upload-artifact") && yaml.contains("download-artifact"));
}

#[test]
fn backend_emit_via_trait() {
    let plan = ariadne::planner::plan(&fixture()).unwrap();
    let backend = GithubActionsBackend::default();
    assert_eq!(backend.name(), "github");
    let yaml = backend.emit(&plan).unwrap();
    assert!(yaml.contains("name: Simple Build and Test"));
}

use ariadne::backends::local::LocalBackend;
use ariadne::testing::{check_plan, optimize_for, run_fixture, Fixture};
use std::fs;
use std::path::Path;
use std::process::Command;

fn podman_available() -> bool {
    Command::new("podman")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Discovers every fixture under tests/fixtures/loom-test and checks it.
/// Plan-level assertions run with NO execution (always, even without podman).
/// Execution-level assertions (stdout / unit pass-fail) run the workflow in
/// podman, and are skipped when podman is unavailable.
#[test]
fn loom_test_fixtures() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/loom-test");
    let podman = podman_available();
    if !podman {
        eprintln!("podman unavailable: evaluating plan-level assertions only");
    }

    let mut ran = 0;
    let mut failures: Vec<String> = Vec::new();

    for entry in fs::read_dir(&dir).expect("fixtures dir exists") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let fixture: Fixture = serde_json::from_str(&fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("parse fixture {name}: {e}"));

        // LocalBackend constructs without running anything; needed for plan-level
        // instruction-selection checks too.
        let backend = LocalBackend::podman();
        let results = if podman {
            run_fixture(&name, &fixture, &backend)
                .unwrap_or_else(|e| panic!("run fixture {name}: {e}"))
                .results
        } else {
            let baseline = ariadne::planner::plan_for(&fixture.workflow, &fixture.event)
                .unwrap_or_else(|e| panic!("plan fixture {name}: {e:?}"));
            let caps = ariadne::backends::derive_capability_profile_from_inventory(
                fixture.workflow.inventory.as_ref()
            );
            let plan = optimize_for(&fixture.workflow, baseline, caps);
            check_plan(&plan, &backend, &fixture.assertions)
        };

        ran += 1;
        for r in results.iter().filter(|r| !r.passed) {
            failures.push(format!("[{name}] {:?}: {}", r.assertion, r.detail));
        }
    }

    assert!(ran > 0, "no fixtures discovered in {}", dir.display());
    assert!(failures.is_empty(), "fixture assertion failures:\n{}", failures.join("\n"));
    eprintln!("loom-test fixtures: {ran} checked (podman={podman})");
}

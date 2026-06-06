use std::path::Path;
use std::process::Command;

fn loom() -> Command {
    Command::new(env!("CARGO_BIN_EXE_loom"))
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn check_valid_fixture_exits_zero() {
    let out = loom().args(["check", &fixture("simple-build-test.tir.json")]).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("valid"));
}

#[test]
fn check_missing_file_exits_nonzero() {
    let out = loom().args(["check", "/no/such/file.json"]).output().unwrap();
    assert!(!out.status.success());
}

#[test]
fn plan_github_produces_yaml() {
    let out = loom()
        .args(["plan", &fixture("simple-build-test.tir.json"), "github"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("name: Simple Build and Test"));
    assert!(stdout.contains("jobs:"));
}

#[test]
fn plan_unknown_backend_exits_nonzero() {
    let out = loom()
        .args(["plan", &fixture("simple-build-test.tir.json"), "bogus"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn explain_shows_units_and_decisions() {
    let out = loom().args(["explain", &fixture("simple-build-test.tir.json")]).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("checkout"));
    assert!(stdout.contains("build"));
    assert!(stdout.contains("test"));
}

#[test]
fn explain_with_backend_shows_instruction_selection() {
    let out = loom()
        .args(["explain", &fixture("simple-build-test.tir.json"), "github"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("instruction selection (github)"), "{stdout}");
    assert!(stdout.contains("github.shell.run"), "{stdout}");
}

#[test]
fn docs_produces_markdown() {
    let out = loom()
        .args(["docs", &fixture("simple-build-test.tir.json")])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# Simple Build and Test"), "{stdout}");
    assert!(stdout.contains("## Summary"), "{stdout}");
}

#[test]
fn docs_with_backend_appends_summary() {
    let out = loom()
        .args(["docs", &fixture("simple-build-test.tir.json"), "github"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("# Simple Build and Test"));
}

#[test]
fn plan_writes_output_file() {
    let out_path = std::env::temp_dir().join(format!("loom-out-{}.yml", std::process::id()));
    let out = loom()
        .args(["plan", &fixture("simple-build-test.tir.json"), "github", "-o"])
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let written = std::fs::read_to_string(&out_path).unwrap();
    assert!(written.contains("jobs:"));
    let _ = std::fs::remove_file(out_path);
}

/// Writes a workflow `.tir.json` + sibling `<base>.test.json` suite in a temp
/// dir and returns the workflow path. The suite is plan-level only, so running
/// it needs no podman.
fn write_suite(base: &str, suite: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("loom-suite-{}-{base}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wf = r#"{"name":"w","artifacts":[{"name":"bin","ty":"Binary","producer":0}],
      "action_calls":[{"name":"deploy","action":"deploy","outputs":[0],"consequences":[0],
        "shell":{"script":"echo x","capture":"NoCapture"}}],
      "consequences":[{"name":"ship","kind":"Deployment","requires_approval":false}],
      "inventory":{"id":"i","actors":[{"id":"l","labels":["ubuntu-latest"]}]},
      "policies":{"max_parallel_jobs":3}}"#;
    std::fs::write(dir.join(format!("{base}.tir.json")), wf).unwrap();
    std::fs::write(dir.join(format!("{base}.test.json")), suite).unwrap();
    dir.join(format!("{base}.tir.json"))
}

#[test]
fn test_suite_plan_level_passes_without_podman() {
    let wf = write_suite("ok", r#"{"cases":[
        {"name":"gated on pr","event":{"pull_request":{"fork":false}},
         "assertions":[{"assert":"consequence_gated","effect":"ship"}]},
        {"name":"policy ok","assertions":[{"assert":"max_parallel_jobs","max":10}]}
    ]}"#);
    let out = loom().arg("test").arg(&wf)
        .env("PATH", "")  // hide podman; plan-level cases must not need it
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout: {stdout}\nstderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("2/2 cases passed"), "{stdout}");
}

#[test]
fn test_suite_detects_policy_violation() {
    let wf = write_suite("bad", r#"{"cases":[
        {"name":"too parallel","assertions":[{"assert":"max_parallel_jobs","max":1}]}
    ]}"#);
    let out = loom().arg("test").arg(&wf).env("PATH", "").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("[FAIL]"));
}

use ariadne::backends::Backend;
use ariadne::backends::local::LocalBackend;
use ariadne::backends::registry::BackendRegistry;
use ariadne::Pipeline;
use clap::{Parser, Subcommand};
use loom::{has_errors, load_workflow, print_diags};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "loom", about = "Loom — a language-agnostic CLI over Thread IR (TIR)")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Is this workflow valid? Validate the TIR; no execution.
    Check { file: PathBuf },
    /// What plan should be produced? Plan and emit backend output.
    Plan {
        file: PathBuf,
        /// Target backend: github | local
        backend: String,
        /// Write the generated output to a file instead of stdout (for
        /// `git diff --exit-code` testing of checked-in CI).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Optimization level: -O0 baseline .. -O3 aggressive (default 2).
        #[arg(short = 'O', default_value_t = 2)]
        opt_level: u8,
        /// Profile data (JSON) to guide optimization.
        #[arg(long)]
        profile: Option<PathBuf>,
    },
    /// Does this workflow behave correctly? Run its test suite (assertions),
    /// or execute once in Podman if no suite is configured.
    Test {
        /// Event to simulate when no test suite is found: push | pr | fork-pr | tag
        #[arg(long, default_value = "push")]
        event: String,
        /// Test suite (cases + assertions). Defaults to `<file>.test.json`.
        #[arg(long)]
        tests: Option<PathBuf>,
        /// Container image to run actions in (default ubuntu:24.04). Use one
        /// with the tools your scripts need, e.g. rust:1 for git + cargo.
        #[arg(long)]
        image: Option<String>,
        /// Directory the repo is mounted at inside the container (default
        /// /workspace).
        #[arg(long)]
        workdir: Option<String>,
        file: PathBuf,
    },
    /// Generate human-readable Markdown documentation from a Thread IR workflow.
    /// Pass a backend id to append a backend summary section.
    Docs {
        file: PathBuf,
        /// Optional backend for the Backend Summary section: github | local
        backend: Option<String>,
    },
    /// Produce a profile from real CI runs to guide optimization. Reads a
    /// backend's run telemetry (GitHub: jobs + artifacts via `gh api`) and
    /// aggregates it into a profile.json the planner can consume with --profile.
    Profile {
        /// Backend whose runs to read: github (default).
        #[arg(default_value = "github")]
        backend: String,
        /// Workflow file name or id to read runs of (e.g. main.yml). Required
        /// unless --from is given.
        #[arg(long)]
        workflow: Option<String>,
        /// owner/repo to read from. Defaults to the current repository (gh).
        #[arg(long)]
        repo: Option<String>,
        /// Number of recent successful runs to aggregate.
        #[arg(long, default_value_t = 5)]
        runs: u32,
        /// Read pre-fetched run telemetry (*.json bundles) from a directory
        /// instead of calling the backend API.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Where to write the profile.
        #[arg(short, long, default_value = "profile.json")]
        out: PathBuf,
    },
    /// Why was this plan selected? Show planner, placement, and optimization
    /// decisions. Pass a backend to also show instruction selection.
    Explain {
        file: PathBuf,
        /// Optional backend for instruction-selection detail: github | local
        backend: Option<String>,
        /// Optimization level: -O0 baseline .. -O3 aggressive (default 2).
        #[arg(short = 'O', default_value_t = 2)]
        opt_level: u8,
        /// Profile data (JSON) to guide optimization.
        #[arg(long)]
        profile: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Check { file } => {
            let wf = load_workflow_or_exit(&file);
            let diags = Pipeline::new(wf).validate();
            print_diags(&diags);
            if has_errors(&diags) { std::process::exit(1); }
            println!("Workflow is valid.");
        }
        Cmd::Plan { file, backend, out, opt_level, profile } => {
            let pipeline = validated_pipeline_or_exit(&file);
            let plan = match pipeline.plan() {
                Ok(plan) => { print_diags(&plan.diagnostics); plan }
                Err(errs) => { print_diags(&errs); std::process::exit(1); }
            };
            let prof = load_profile(profile.as_deref());
            let mut registry = BackendRegistry::with_builtins();
            let b = registry.resolve_or_die(&backend);
            let caps = ariadne::backends::derive_capability_profile_from_inventory(
                pipeline.workflow.inventory.as_ref()
            );
            let plan = optimize_plan(&pipeline.workflow, plan, caps, opt_level, &prof);
            let output = b.emit(&plan).unwrap_or_else(|errs| { print_diags(&errs); std::process::exit(1); });
            match out {
                Some(path) => {
                    if let Err(e) = std::fs::write(&path, &output) {
                        eprintln!("error: cannot write '{}': {e}", path.display());
                        std::process::exit(1);
                    }
                    eprintln!("wrote {}", path.display());
                }
                None => print!("{output}"),
            }
        }
        Cmd::Docs { file, backend } => {
            let wf = load_workflow_or_exit(&file);
            let output = if let Some(ref b_id) = backend {
                let mut registry = BackendRegistry::with_builtins();
                let b = registry.resolve_or_die(b_id);
                ariadne::docs::generate_with_backend(&wf, b)
            } else {
                ariadne::docs::generate(&wf)
            };
            print!("{output}");
        }
        Cmd::Explain { file, backend, opt_level, profile } => {
            let pipeline = validated_pipeline_or_exit(&file);
            let plan = match pipeline.plan() {
                Ok(plan) => plan,
                Err(errs) => { print_diags(&errs); std::process::exit(1); }
            };
            let prof = load_profile(profile.as_deref());
            let mut registry = BackendRegistry::with_builtins();
            let caps = ariadne::backends::derive_capability_profile_from_inventory(
                pipeline.workflow.inventory.as_ref()
            );
            let plan = optimize_plan(&pipeline.workflow, plan, caps, opt_level, &prof);
            print_explain(&plan, &prof);
            if let Some(ref b_id) = backend {
                if let Some(b) = registry.resolve(b_id) {
                    explain_selection_dyn(&plan, b);
                } else {
                    eprintln!("warning: unknown backend '{b_id}' — skipping instruction selection");
                }
            }
        }
        Cmd::Profile { backend, workflow, repo, runs, from, out } => {
            let registry = ariadne::telemetry::CollectorRegistry::with_builtins();
            let Some(collector) = registry.resolve(&backend) else {
                eprintln!("error: no profile collector for backend '{backend}'; have: {}",
                    registry.ids().join(", "));
                std::process::exit(1);
            };
            let raw = match gather_runs(&backend, workflow.as_deref(), repo.as_deref(), runs, from.as_deref()) {
                Ok(r) if r.is_empty() => { eprintln!("error: no run telemetry found"); std::process::exit(1); }
                Ok(r) => r,
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            };
            let reports = collector.parse(&raw).unwrap_or_else(|e| {
                eprintln!("error: {e}"); std::process::exit(1);
            });
            let profile = ariadne::telemetry::aggregate(&reports, &collector.runner_pricing());
            let json = serde_json::to_string_pretty(&profile).expect("serialize profile");
            if let Err(e) = std::fs::write(&out, json) {
                eprintln!("error: cannot write '{}': {e}", out.display());
                std::process::exit(1);
            }
            eprintln!("wrote {} from {} run(s)", out.display(), reports.len());
        }
        Cmd::Test { event, tests, image, workdir, file } => {
            let pipeline = validated_pipeline_or_exit(&file);
            let mut backend = LocalBackend::podman();
            if let Some(i) = image { backend = backend.with_image(i); }
            if let Some(w) = workdir { backend = backend.with_workdir(w); }
            let suite_path = tests.unwrap_or_else(|| sidecar_suite(&file));
            if suite_path.exists() {
                run_suite(&pipeline.workflow, &suite_path, &backend);
            } else {
                use ariadne::testing::Executor;
                let ctx = parse_event(&event);
                let plan = match ariadne::planner::plan_for(&pipeline.workflow, &ctx) {
                    Ok(plan) => { print_diags(&plan.diagnostics); plan }
                    Err(errs) => { print_diags(&errs); std::process::exit(1); }
                };
                match backend.execute(&plan) {
                    Ok(run) => {
                        print_test_run(&run);
                        if !run.passed() { std::process::exit(1); }
                    }
                    Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
                }
            }
        }
    }
}

/// Default test-suite path for a workflow: the base name (before any extension)
/// plus `.test.json`. So `release.tir.pb` -> `release.test.json`.
fn sidecar_suite(file: &std::path::Path) -> PathBuf {
    let name = file.file_name().and_then(|s| s.to_str()).unwrap_or("workflow");
    let base = name.split('.').next().unwrap_or("workflow");
    file.with_file_name(format!("{base}.test.json"))
}

fn run_suite<E: Backend + ariadne::testing::Executor>(
    workflow: &ariadne::ir::Workflow,
    suite_path: &std::path::Path,
    backend: &E,
) {
    use ariadne::testing::{run_case, TestSuite};
    let suite = TestSuite::load(suite_path)
        .unwrap_or_else(|e| { eprintln!("error: {e}"); std::process::exit(1); });

    let mut failed = 0;
    println!("Test suite: {} case(s)", suite.cases.len());
    for case in &suite.cases {
        match run_case(workflow, case, backend) {
            Ok(report) => {
                let fails: Vec<_> = report.failures().collect();
                if fails.is_empty() {
                    println!("  [PASS] {} ({} assertions)", case.name, report.results.len());
                } else {
                    failed += 1;
                    println!("  [FAIL] {}", case.name);
                    for f in fails {
                        println!("      {:?}: {}", f.assertion, f.detail);
                    }
                }
            }
            Err(e) => { failed += 1; println!("  [ERR ] {}: {e}", case.name); }
        }
    }
    println!("{}/{} cases passed", suite.cases.len() - failed, suite.cases.len());
    if failed > 0 { std::process::exit(1); }
}

/// Gather raw per-run telemetry for a backend: from a directory of pre-fetched
/// JSON bundles (`--from`), else by querying the backend's API.
fn gather_runs(
    backend: &str,
    workflow: Option<&str>,
    repo: Option<&str>,
    runs: u32,
    from: Option<&std::path::Path>,
) -> Result<Vec<ariadne::telemetry::RawRun>, String> {
    use ariadne::telemetry::RawRun;
    if let Some(dir) = from {
        let mut out = Vec::new();
        let entries = std::fs::read_dir(dir).map_err(|e| format!("read dir '{}': {e}", dir.display()))?;
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        paths.sort();
        for p in paths {
            out.push(RawRun(std::fs::read_to_string(&p).map_err(|e| format!("read '{}': {e}", p.display()))?));
        }
        return Ok(out);
    }
    match backend {
        "github" => fetch_github_runs(workflow.ok_or("--workflow is required for github (e.g. --workflow main.yml)")?, repo, runs),
        other => Err(format!("fetching runs for backend '{other}' is not supported; use --from <dir>")),
    }
}

/// Fetch the most recent successful runs of a GitHub workflow and bundle each
/// run's jobs + artifacts responses into one JSON payload the collector parses.
/// Uses the `gh` CLI so auth is handled by the user's existing login.
fn fetch_github_runs(workflow: &str, repo: Option<&str>, runs: u32) -> Result<Vec<ariadne::telemetry::RawRun>, String> {
    use ariadne::telemetry::RawRun;
    // `gh api` substitutes {owner}/{repo} from the current repository.
    let r = repo.unwrap_or("{owner}/{repo}");
    let ids_json = gh(&[
        "api".into(),
        format!("repos/{r}/actions/workflows/{workflow}/runs?status=success&per_page={runs}"),
        "--jq".into(), ".workflow_runs[].id".into(),
    ])?;
    let ids: Vec<&str> = ids_json.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if ids.is_empty() {
        return Err(format!("no successful runs found for workflow '{workflow}'"));
    }
    let mut out = Vec::new();
    for id in ids {
        let jobs = gh(&["api".into(), format!("repos/{r}/actions/runs/{id}/jobs?per_page=100")])?;
        let artifacts = gh(&["api".into(), format!("repos/{r}/actions/runs/{id}/artifacts?per_page=100")])?;
        out.push(RawRun(format!("{{\"jobs\":{jobs},\"artifacts\":{artifacts}}}")));
    }
    Ok(out)
}

/// Run `gh` with the given args, returning stdout or a friendly error.
fn gh(args: &[String]) -> Result<String, String> {
    let output = std::process::Command::new("gh").args(args).output()
        .map_err(|e| format!("failed to run gh: {e} (install the GitHub CLI: https://cli.github.com)"))?;
    if !output.status.success() {
        return Err(format!("gh api failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn load_profile(profile: Option<&std::path::Path>) -> ariadne::profile::Profile {
    use ariadne::profile::Profile;
    match profile {
        Some(p) => Profile::load(p).unwrap_or_else(|e| { eprintln!("error: {e}"); std::process::exit(1) }),
        None => Profile::default(),
    }
}

fn optimize_plan(
    wf: &ariadne::ir::Workflow,
    plan: ariadne::planner::Plan,
    caps: ariadne::backends::BackendCapabilities,
    opt_level: u8,
    prof: &ariadne::profile::Profile,
) -> ariadne::planner::Plan {
    use ariadne::analysis::Analysis;
    use ariadne::optimize::{optimize, OptLevel, OptimizeCtx};
    let analysis = Analysis::of(wf);
    let ctx = OptimizeCtx {
        workflow: wf,
        profile: prof,
        backend_caps: caps,
        policy: &wf.policies,
        analysis: &analysis,
        objectives: wf.policies.objectives.clone(),
        level: OptLevel::from_u8(opt_level),
    };
    optimize(plan, &ctx)
}

fn parse_event(s: &str) -> ariadne::testing::EventContext {
    use ariadne::testing::EventContext;
    match s {
        "pr" => EventContext::PullRequest { fork: false },
        "fork-pr" => EventContext::PullRequest { fork: true },
        "tag" => EventContext::Tag { name: "v0.0.0".into() },
        _ => EventContext::Push { branch: "main".into() },
    }
}

fn print_test_run(run: &ariadne::testing::TestRun) {
    use ariadne::testing::UnitStatus;
    println!("Test run: {}", run.workflow_name);
    for unit in &run.units {
        let mark = match unit.status {
            UnitStatus::Passed => "PASS",
            UnitStatus::Failed(_) => "FAIL",
            UnitStatus::Skipped => "SKIP",
        };
        println!("  [{mark}] {}", unit.action_name);
        if !unit.artifacts.is_empty() {
            println!("    artifacts: {}", unit.artifacts.join(", "));
        }
        for t in &unit.transfers {
            println!("    transfer: {} ({:?})", t.name, t.kind);
        }
        if !unit.secrets_spoofed.is_empty() {
            println!("    secrets (spoofed): {}", unit.secrets_spoofed.join(", "));
        }
        if !unit.secrets_withheld.is_empty() {
            println!("    secrets (withheld): {}", unit.secrets_withheld.join(", "));
        }
        for e in &unit.consequences_fired {
            println!("    effect would fire: {} ({:?})", e.name, e.kind);
        }
        for e in &unit.consequences_gated {
            println!("    effect gated: {} ({:?})", e.name, e.kind);
        }
        if let UnitStatus::Failed(code) = unit.status {
            println!("    exit code: {code}");
            if !unit.stderr.is_empty() {
                println!("    stderr:\n{}", indent(&unit.stderr));
            }
        }
    }
    let passed = run.units.iter().filter(|u| u.passed()).count();
    println!("{passed}/{} units passed", run.units.len());
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("      {l}")).collect::<Vec<_>>().join("\n")
}

fn op_label(op: &ariadne::planner::LogicalOp) -> String {
    use ariadne::planner::{AccessMode, LogicalOp};
    match op {
        LogicalOp::CheckoutRepo => "checkout repository".to_string(),
        LogicalOp::RunShell { label, .. } => format!("run shell: {label}"),
        LogicalOp::UploadArtifact { name, .. } => format!("upload artifact '{name}' (copy)"),
        LogicalOp::DownloadArtifact { name, .. } => format!("download artifact '{name}' (copy fallback)"),
        LogicalOp::TransferArtifact { name, access, .. } => {
            let how = match access {
                AccessMode::Copy => "copy",
                AccessMode::MountReadOnly => "mount (ro)",
                AccessMode::MountReadWrite => "mount (rw)",
                AccessMode::Stream => "stream",
                AccessMode::SameHostPath => "same-host path",
                AccessMode::OciLayer => "oci layer",
            };
            format!("transfer artifact '{name}' via {how}")
        }
        LogicalOp::RestoreCache { key } => format!("restore cache '{key}'"),
        LogicalOp::SaveCache { key } => format!("save cache '{key}'"),
        LogicalOp::RequestApproval { reason } => format!("request approval: {reason}"),
        LogicalOp::Native { id, fallback, .. } => {
            format!("{id} (native step, fallback: {fallback})")
        }
    }
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[i]) }
}

fn print_explain(plan: &ariadne::planner::Plan, profile: &ariadne::profile::Profile) {
    use ariadne::cost::Cost;
    println!("Plan: {}", plan.workflow_name);
    println!("  {} execution unit(s)", plan.units.len());
    println!("  peak concurrency: {}", plan.max_concurrency());
    if let Some(max) = plan.max_parallel_jobs {
        println!("  max_parallel_jobs: {max}");
    }

    let cost = Cost::estimate(plan, profile);
    println!("  estimated cost: makespan {:.1}s, transfer {}, ${:.4}",
        cost.seconds, human_bytes(cost.transfer_bytes), cost.dollars);

    for unit in &plan.units {
        println!("\n  [{}] {} (runner: {})", unit.id, unit.action_name, unit.runner);
        if !unit.needs.is_empty() {
            println!("    needs: {}", unit.needs.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", "));
        }
        for op in &unit.ops {
            println!("    - {}", op_label(op));
        }
        for e in &unit.consequences {
            println!("    effect: {} ({:?}){}", e.name, e.kind,
                if e.requires_approval { " [requires approval]" } else { "" });
        }
    }

    if !plan.optimizations.is_empty() {
        println!("\n  optimizations:");
        for o in &plan.optimizations {
            println!("    [{}] {}: {} -> {} ({})", o.pass, o.target, o.from, o.to, o.reason);
        }
    }

    if !plan.diagnostics.is_empty() {
        println!("\n  decisions & warnings:");
        for d in &plan.diagnostics {
            println!("    {d}");
        }
    }
}

fn explain_selection_dyn(plan: &ariadne::planner::Plan, backend: &dyn ariadne::backends::EmittingBackend) {
    println!("\n  instruction selection ({}):", backend.id());
    for unit in &plan.units {
        println!("    [{}]", unit.id);
        for op in &unit.ops {
            match backend.select_op(op) {
                Some(sel) => println!("      {} -> {} ({})", op_label(op), sel.id, sel.reason),
                None => println!("      {} -> (no instruction available)", op_label(op)),
            }
        }
    }
}

fn load_workflow_or_exit(path: &std::path::Path) -> ariadne::ir::Workflow {
    load_workflow(path).unwrap_or_else(|e| { eprintln!("error: {e}"); std::process::exit(1); })
}

fn validated_pipeline_or_exit(path: &std::path::Path) -> Pipeline {
    let pipeline = Pipeline::new(load_workflow_or_exit(path));
    let diags = pipeline.validate();
    print_diags(&diags);
    if has_errors(&diags) { std::process::exit(1); }
    pipeline
}

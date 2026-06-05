use ariadne::backends::Backend;
use ariadne::backends::local::LocalBackend;
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
        file: PathBuf,
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
            use ariadne::backends::github::GithubActionsBackend;
            let pipeline = validated_pipeline_or_exit(&file);
            let plan = match pipeline.plan() {
                Ok(plan) => { print_diags(&plan.diagnostics); plan }
                Err(errs) => { print_diags(&errs); std::process::exit(1); }
            };
            let prof = load_profile(profile.as_deref());
            let output = match backend.as_str() {
                "github" => {
                    let b = GithubActionsBackend::default();
                    let plan = optimize_plan(&pipeline.workflow, plan, b.capability_profile(), opt_level, &prof);
                    emit_or_exit(b.emit(&plan))
                }
                "local" => {
                    let b = LocalBackend::podman();
                    let plan = optimize_plan(&pipeline.workflow, plan, b.capability_profile(), opt_level, &prof);
                    emit_or_exit(b.emit(&plan))
                }
                other => {
                    eprintln!("error: unknown backend '{other}'. Supported: github, local");
                    std::process::exit(1);
                }
            };
            match out {
                Some(path) => {
                    if let Err(e) = std::fs::write(&path, output) {
                        eprintln!("error: cannot write '{}': {e}", path.display());
                        std::process::exit(1);
                    }
                    eprintln!("wrote {}", path.display());
                }
                None => print!("{output}"),
            }
        }
        Cmd::Explain { file, backend, opt_level, profile } => {
            use ariadne::backends::github::GithubActionsBackend;
            let pipeline = validated_pipeline_or_exit(&file);
            let plan = match pipeline.plan() {
                Ok(plan) => plan,
                Err(errs) => { print_diags(&errs); std::process::exit(1); }
            };
            let caps = match backend.as_deref() {
                Some("github") => GithubActionsBackend::default().capability_profile(),
                Some("local") => LocalBackend::podman().capability_profile(),
                _ => ariadne::backends::BackendCapabilities::default(),
            };
            let prof = load_profile(profile.as_deref());
            let plan = optimize_plan(&pipeline.workflow, plan, caps, opt_level, &prof);
            print_explain(&plan, &prof);
            if let Some(b) = backend {
                explain_selection(&plan, &b);
            }
        }
        Cmd::Test { event, tests, file } => {
            let pipeline = validated_pipeline_or_exit(&file);
            let suite_path = tests.unwrap_or_else(|| sidecar_suite(&file));
            if suite_path.exists() {
                run_suite(&pipeline.workflow, &suite_path);
            } else {
                use ariadne::testing::Executor;
                let ctx = parse_event(&event);
                let plan = match ariadne::planner::plan_for(&pipeline.workflow, &ctx) {
                    Ok(plan) => { print_diags(&plan.diagnostics); plan }
                    Err(errs) => { print_diags(&errs); std::process::exit(1); }
                };
                match LocalBackend::podman().execute(&plan) {
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

fn run_suite(workflow: &ariadne::ir::Workflow, suite_path: &std::path::Path) {
    use ariadne::backends::local::LocalBackend;
    use ariadne::testing::{run_case, TestSuite};
    let suite = TestSuite::load(suite_path)
        .unwrap_or_else(|e| { eprintln!("error: {e}"); std::process::exit(1); });

    let backend = LocalBackend::podman();
    let mut failed = 0;
    println!("Test suite: {} case(s)", suite.cases.len());
    for case in &suite.cases {
        match run_case(workflow, case, &backend) {
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

fn emit_or_exit(r: Result<String, Vec<ariadne::diagnostics::Diagnostic>>) -> String {
    match r {
        Ok(s) => s,
        Err(errs) => { print_diags(&errs); std::process::exit(1); }
    }
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
        for e in &unit.effects_fired {
            println!("    effect would fire: {} ({:?})", e.name, e.kind);
        }
        for e in &unit.effects_gated {
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

fn op_label(op: &ariadne::planner::PhysicalOp) -> String {
    use ariadne::planner::{AccessMode, PhysicalOp};
    match op {
        PhysicalOp::CheckoutRepo => "checkout repository".to_string(),
        PhysicalOp::RunShell { label, .. } => format!("run shell: {label}"),
        PhysicalOp::UploadArtifact { name, .. } => format!("upload artifact '{name}' (copy)"),
        PhysicalOp::DownloadArtifact { name, .. } => format!("download artifact '{name}' (copy fallback)"),
        PhysicalOp::TransferArtifact { name, access, .. } => {
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
        PhysicalOp::RestoreCache { key } => format!("restore cache '{key}'"),
        PhysicalOp::SaveCache { key } => format!("save cache '{key}'"),
        PhysicalOp::RequestApproval { reason } => format!("request approval: {reason}"),
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
    println!("  estimated cost: critical path {:.1}s, transfer {}, ${:.4}",
        cost.seconds, human_bytes(cost.transfer_bytes), cost.dollars);

    for unit in &plan.units {
        println!("\n  [{}] {} (runner: {})", unit.id, unit.action_name, unit.runner);
        if !unit.needs.is_empty() {
            println!("    needs: {}", unit.needs.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", "));
        }
        for op in &unit.ops {
            println!("    - {}", op_label(op));
        }
        for e in &unit.effects {
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

fn explain_selection(plan: &ariadne::planner::Plan, backend: &str) {
    use ariadne::backends::github::GithubActionsBackend;
    match backend {
        "github" => selection_report(plan, &GithubActionsBackend::default()),
        "local" => selection_report(plan, &LocalBackend::podman()),
        other => {
            eprintln!("error: unknown backend '{other}'. Supported: github, local");
            std::process::exit(1);
        }
    }
}

fn selection_report<B: Backend>(plan: &ariadne::planner::Plan, backend: &B) {
    use ariadne::backends::Selector;
    let selector = Selector::for_backend(backend);
    let caps = backend.capabilities();
    println!("\n  instruction selection ({}):", backend.name());
    for unit in &plan.units {
        println!("    [{}]", unit.id);
        for op in &unit.ops {
            match selector.select(op, &caps, &[]) {
                Some(sel) => println!("      {} -> {} ({})", op_label(op), sel.instruction.id.0, sel.reason),
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

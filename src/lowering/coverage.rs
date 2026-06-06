use super::{arg_list, arg_str, def, local, Registry};

/// Code-coverage lowerings.
pub fn register(r: &mut Registry) {
    r.register(def("coverage.measure.cargo", "coverage.measure", "cargo", |a| {
        let out = arg_str(a, "out").unwrap_or_else(|| "lcov.info".into());
        local(vec![
            "cargo".into(), "llvm-cov".into(), "--workspace".into(),
            "--lcov".into(), "--output-path".into(), out,
        ])
    }));
    r.register(def("coverage.measure.pytest", "coverage.measure", "pytest", |a| {
        let mut p = vec!["pytest".into()];
        p.extend(arg_list(a, "paths"));
        if let Some(pkg) = arg_str(a, "package") {
            p.push(format!("--cov={pkg}"));
        }
        let out = arg_str(a, "out").unwrap_or_else(|| "coverage.xml".into());
        p.push(format!("--cov-report=xml:{out}"));
        local(p)
    }));
}

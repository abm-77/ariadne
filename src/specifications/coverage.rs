use super::{Registry, arg_list, arg_str, def, local};

/// Code-coverage lowerings.
pub fn register(r: &mut Registry) {
    r.register(
        def("coverage.measure.cargo", "coverage.measure", "cargo", |a| {
            let out = arg_str(a, "out").unwrap_or_else(|| "lcov.info".into());
            local(vec![
                "cargo".into(),
                "llvm-cov".into(),
                "--workspace".into(),
                "--lcov".into(),
                "--output-path".into(),
                out,
            ])
        })
        .with_deps(&["cargo-llvm-cov"]),
    );
    r.register(
        def(
            "coverage.measure.pytest",
            "coverage.measure",
            "pytest",
            |a| {
                let mut p = vec!["pytest".into()];
                p.extend(arg_list(a, "paths"));
                if let Some(pkg) = arg_str(a, "package") {
                    p.push(format!("--cov={pkg}"));
                }
                let out = arg_str(a, "out").unwrap_or_else(|| "coverage.xml".into());
                p.push(format!("--cov-report=xml:{out}"));
                local(p)
            },
        )
        .with_deps(&["pytest", "pytest-cov"]),
    );
}

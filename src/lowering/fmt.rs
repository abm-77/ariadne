use super::{Registry, arg_list, def, local};

/// Format-check lowerings.
pub fn register(r: &mut Registry) {
    r.register(def("fmt.check.cargo", "fmt.check", "cargo", |_| {
        local(vec![
            "cargo".into(),
            "fmt".into(),
            "--all".into(),
            "--".into(),
            "--check".into(),
        ])
    }));
    r.register(
        def("fmt.check.ruff", "fmt.check", "ruff", |a| {
            let mut p = vec!["ruff".into(), "format".into(), "--check".into()];
            p.extend(arg_list(a, "paths"));
            local(p)
        })
        .with_deps(&["ruff"]),
    );
}

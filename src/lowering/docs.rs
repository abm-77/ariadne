use super::{Registry, arg_str, def, local};

/// Documentation-generation lowerings.
pub fn register(r: &mut Registry) {
    r.register(def("docs.generate.cargo", "docs.generate", "cargo", |_| {
        local(vec![
            "cargo".into(),
            "doc".into(),
            "--no-deps".into(),
            "--workspace".into(),
        ])
    }));
    r.register(
        def("docs.generate.pdoc", "docs.generate", "pdoc", |a| {
            let out = arg_str(a, "out").unwrap_or_else(|| "docs".into());
            let pkg = arg_str(a, "package").unwrap_or_default();
            local(vec!["pdoc".into(), "-o".into(), out, pkg])
        })
        .with_deps(&["pdoc"]),
    );
    r.register(
        def("docs.generate.mkdocs", "docs.generate", "mkdocs", |a| {
            let site = arg_str(a, "out").unwrap_or_else(|| "site".into());
            local(vec![
                "mkdocs".into(),
                "build".into(),
                "--site-dir".into(),
                site,
            ])
        })
        .with_deps(&["mkdocs"]),
    );
}

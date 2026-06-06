use super::{Registry, arg_flag, arg_str, def, local};

/// Build lowerings: binaries, libraries, wheels, container images, docs.
pub fn register(r: &mut Registry) {
    r.register(def("build.binary.cargo", "build.binary", "cargo", |a| {
        let mut p = vec!["cargo".into(), "build".into()];
        if arg_flag(a, "release") {
            p.push("--release".into());
        }
        if let Some(pkg) = arg_str(a, "package") {
            p.push("--package".into());
            p.push(pkg);
        }
        p.extend(super::arg_list(a, "args"));
        local(p)
    }));
    r.register(def("build.library.cargo", "build.library", "cargo", |a| {
        let mut p = vec!["cargo".into(), "build".into(), "--lib".into()];
        if arg_flag(a, "release") {
            p.push("--release".into());
        }
        if let Some(pkg) = arg_str(a, "package") {
            p.push("--package".into());
            p.push(pkg);
        }
        p.extend(super::arg_list(a, "args"));
        local(p)
    }));
    r.register(
        def(
            "build.python_wheel.maturin",
            "build.python_wheel",
            "maturin",
            |a| {
                let mut p: Vec<String> = Vec::new();
                // `dir` runs maturin from a project directory (its pyproject
                // decides packaging, e.g. python-source); otherwise build the
                // crate at `manifest`.
                if let Some(d) = arg_str(a, "dir") {
                    p.push(format!("cd {d} &&"));
                }
                p.push("maturin".into());
                p.push("build".into());
                if arg_flag(a, "release") {
                    p.push("--release".into());
                }
                if let Some(m) = arg_str(a, "manifest") {
                    p.push("--manifest-path".into());
                    p.push(m);
                }
                p.push("--out".into());
                p.push(arg_str(a, "out").unwrap_or_else(|| "dist".into()));
                local(p)
            },
        )
        .with_deps(&["maturin"]),
    );
    r.register(
        def("build.python_wheel.uv", "build.python_wheel", "uv", |a| {
            let out = arg_str(a, "out").unwrap_or_else(|| "dist".into());
            local(vec![
                "uv".into(),
                "build".into(),
                "--wheel".into(),
                "--out-dir".into(),
                out,
            ])
        })
        .with_deps(&["uv"]),
    );
    r.register(def(
        "build.python_wheel.python-build",
        "build.python_wheel",
        "python",
        |a| {
            let out = arg_str(a, "out").unwrap_or_else(|| "dist".into());
            local(vec![
                "python".into(),
                "-m".into(),
                "build".into(),
                "--wheel".into(),
                "--outdir".into(),
                out,
            ])
        },
    ));
    r.register(def(
        "build.container_image.buildkit",
        "build.container_image",
        "buildkit",
        |a| {
            let tag = arg_str(a, "tag").unwrap_or_default();
            local(vec![
                "buildctl".into(),
                "build".into(),
                "--frontend".into(),
                "dockerfile.v0".into(),
                "--local".into(),
                "context=.".into(),
                "--local".into(),
                "dockerfile=.".into(),
                "--output".into(),
                format!("type=image,name={tag}"),
            ])
        },
    ));
    r.register(def(
        "build.container_image.docker",
        "build.container_image",
        "docker",
        |a| {
            let tag = arg_str(a, "tag").unwrap_or_default();
            local(vec![
                "docker".into(),
                "build".into(),
                "-t".into(),
                tag,
                ".".into(),
            ])
        },
    ));
    r.register(def(
        "build.container_image.podman",
        "build.container_image",
        "podman",
        |a| {
            let tag = arg_str(a, "tag").unwrap_or_default();
            local(vec![
                "podman".into(),
                "build".into(),
                "-t".into(),
                tag,
                ".".into(),
            ])
        },
    ));
}

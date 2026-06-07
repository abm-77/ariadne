use super::{LoweringBody, Registry, arg_str, def, local};
use std::collections::BTreeMap;

/// Package lowerings: publishing and installing. Publishing is a portable Native
/// op (shell fallback `twine upload`, upgradable to the pypa action). Installing
/// is `package.install`, one lowering per manager (pip, cargo, npm, apt, dnf);
/// selection picks the manager from the call's pin or the inventory, so a tool's
/// install command comes from the same machinery as any other action and a
/// system package lowers to apt or dnf per the actor.
pub fn register(r: &mut Registry) {
    r.register(def(
        "package.publish.twine",
        "package.publish",
        "twine",
        |a| {
            let dist = arg_str(a, "dist").unwrap_or_else(|| "dist/*".into());
            let mut args = BTreeMap::new();
            if let Some(repo) = arg_str(a, "registry") {
                args.insert("repository".into(), repo);
            }
            LoweringBody::Native {
                args,
                fallback: format!("twine upload {dist}"),
            }
        },
    ));

    // apt is registered before dnf, so it is the default system manager when the
    // inventory does not say otherwise.
    r.register(def("package.install.pip", "package.install", "pip", |a| {
        local(vec![
            "pip".into(),
            "install".into(),
            arg_str(a, "package").unwrap_or_default(),
        ])
    }));
    r.register(def(
        "package.install.cargo",
        "package.install",
        "cargo",
        |a| {
            local(vec![
                "cargo".into(),
                "install".into(),
                arg_str(a, "package").unwrap_or_default(),
            ])
        },
    ));
    r.register(def("package.install.npm", "package.install", "npm", |a| {
        local(vec![
            "npm".into(),
            "install".into(),
            "-g".into(),
            arg_str(a, "package").unwrap_or_default(),
        ])
    }));
    r.register(def("package.install.apt", "package.install", "apt", |a| {
        local(vec![
            "sudo".into(),
            "apt-get".into(),
            "install".into(),
            "-y".into(),
            arg_str(a, "package").unwrap_or_default(),
        ])
    }));
    r.register(def("package.install.dnf", "package.install", "dnf", |a| {
        local(vec![
            "sudo".into(),
            "dnf".into(),
            "install".into(),
            "-y".into(),
            arg_str(a, "package").unwrap_or_default(),
        ])
    }));
}

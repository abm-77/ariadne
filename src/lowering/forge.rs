use super::{arg_list, arg_str, def, local, Registry};

/// Forge (code-hosting platform) lowerings.
pub fn register(r: &mut Registry) {
    r.register(def("forge.github.gh", "forge.github", "gh", |a| {
        let tag = arg_str(a, "tag").unwrap_or_default();
        let mut p = vec!["gh".into(), "release".into(), "create".into(), tag];
        p.extend(arg_list(a, "files"));
        local(p)
    }));
}

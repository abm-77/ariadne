use super::{Registry, SpecificationBody, arg_flag, arg_list, arg_str, def, local};

/// Source-control lowerings. `scm.checkout` is a portable Native op: the shell
/// fallback is `git checkout .`, which any backend can run; GitHub upgrades it
/// to `actions/checkout@v4` at emit time via its catalogue. `scm.commit` stages
/// the given paths and commits (and optionally pushes) them via git.
pub fn register(r: &mut Registry) {
    r.register(def("scm.checkout.git", "scm.checkout", "git", |_| {
        SpecificationBody::Native {
            args: Default::default(),
            fallback: "git checkout .".into(),
        }
    }));
    r.register(def("scm.commit.git", "scm.commit", "git", |a| {
        let paths = arg_list(a, "paths").join(" ");
        let message = arg_str(a, "message").unwrap_or_else(|| "update".into());
        // Identity + a no-op-when-clean commit; push when asked. Idempotent so a
        // run with nothing to commit succeeds.
        let mut cmd = format!(
            "git config user.name github-actions && \
             git config user.email github-actions@github.com && \
             git add {paths} && \
             (git diff --staged --quiet || git commit -m \"{message}\")"
        );
        if arg_flag(a, "push") {
            cmd.push_str(" && git push");
        }
        local(vec![cmd])
    }));
}

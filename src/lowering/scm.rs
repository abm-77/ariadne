use super::{def, LoweringBody, Registry};

/// Source-control lowerings. `scm.checkout` is a portable Native op: the shell
/// fallback is `git checkout .`, which any backend can run; GitHub upgrades it
/// to `actions/checkout@v4` at emit time via its catalogue.
pub fn register(r: &mut Registry) {
    r.register(def("scm.checkout.git", "scm.checkout", "git", |_| LoweringBody::Native {
        id: "scm.checkout".into(),
        args: Default::default(),
        fallback: "git checkout .".into(),
    }));
}

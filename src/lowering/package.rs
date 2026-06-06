use super::{arg_str, def, LoweringBody, Registry};
use std::collections::BTreeMap;

/// Package-publishing lowerings. Publishing is a portable Native op: the shell
/// fallback is `twine upload`, which any backend can run. A backend may upgrade
/// it to a native step (e.g. the pypa GitHub Action) at emit time via its
/// catalogue when the inventory permits.
pub fn register(r: &mut Registry) {
    r.register(def("package.publish.twine", "package.publish", "twine", |a| {
        let dist = arg_str(a, "dist").unwrap_or_else(|| "dist/*".into());
        let mut args = BTreeMap::new();
        if let Some(repo) = arg_str(a, "registry") {
            args.insert("repository".into(), repo);
        }
        LoweringBody::Native {
            id: "package.publish".into(),
            args,
            fallback: format!("twine upload {dist}"),
        }
    }));
}

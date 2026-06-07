use super::{Registry, arg_str, def, local};

/// Artifact-signing lowerings.
pub fn register(r: &mut Registry) {
    r.register(def(
        "sign.artifact.cosign",
        "sign.artifact",
        "cosign",
        |a| {
            local(vec![
                "cosign".into(),
                "sign".into(),
                "--yes".into(),
                arg_str(a, "image").unwrap_or_default(),
            ])
        },
    ));
}

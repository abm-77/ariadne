use super::{arg_list, def, local, Registry};

/// Test lowerings. The same `test.unit` action lowers to `cargo test` or
/// `pytest` purely by which runner the inventory declares.
pub fn register(r: &mut Registry) {
    for (id, action) in [
        ("test.unit.cargo", "test.unit"),
        ("test.integration.cargo", "test.integration"),
    ] {
        r.register(def(id, action, "cargo", |a| {
            let mut p = vec!["cargo".into(), "test".into()];
            p.extend(arg_list(a, "args"));
            local(p)
        }));
    }
    for (id, action) in [
        ("test.unit.pytest", "test.unit"),
        ("test.integration.pytest", "test.integration"),
    ] {
        r.register(def(id, action, "pytest", |a| {
            let mut p = vec!["pytest".into()];
            p.extend(arg_list(a, "paths"));
            p.extend(arg_list(a, "args"));
            local(p)
        }).with_deps(&["pytest"]));
    }
}

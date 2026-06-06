use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::EmittingBackend;

pub struct BackendRegistry {
    backends: HashMap<String, Box<dyn EmittingBackend>>,
}

impl BackendRegistry {
    /// Build a registry pre-populated with the two built-in backends.
    pub fn with_builtins() -> Self {
        use crate::backends::github::GithubActionsBackend;
        use crate::backends::local::LocalBackend;
        let mut r = Self { backends: HashMap::new() };
        r.insert(Box::new(GithubActionsBackend::default()));
        r.insert(Box::new(LocalBackend::podman()));
        r
    }

    fn insert(&mut self, b: Box<dyn EmittingBackend>) {
        self.backends.insert(b.id().to_string(), b);
    }

    pub fn builtin_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.backends.keys().map(String::as_str).collect();
        ids.sort();
        ids
    }

    /// Resolve a backend by id. Checks built-ins first, then PATH for
    /// `ariadne-backend-<id>`. Returns `None` if unresolvable.
    pub fn resolve(&mut self, id: &str) -> Option<&dyn EmittingBackend> {
        if !self.backends.contains_key(id) {
            let bin = format!("ariadne-backend-{id}");
            if let Some(path) = find_in_path(&bin) {
                let ext = crate::backends::external::ExternalBackend::load(id, path);
                match ext {
                    Ok(b) => { self.backends.insert(id.to_string(), Box::new(b)); }
                    Err(e) => {
                        eprintln!("warning: found {bin} in PATH but failed to load: {e}");
                        return None;
                    }
                }
            }
        }
        self.backends.get(id).map(|b| b.as_ref())
    }

    pub fn resolve_or_die(&mut self, id: &str) -> &dyn EmittingBackend {
        if self.resolve(id).is_none() {
            let builtins = self.builtin_ids().join(", ");
            eprintln!(
                "error: unknown backend '{id}'. Built-ins: {builtins}. \
                 External backends are discovered as `ariadne-backend-{id}` on PATH."
            );
            std::process::exit(1);
        }
        self.backends[id].as_ref()
    }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file() && is_executable(p))
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_p: &Path) -> bool { true }

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::WorkflowCapabilities;

    #[test]
    fn builtins_include_github_and_local() {
        let mut r = BackendRegistry::with_builtins();
        assert!(r.resolve("github").is_some());
        assert!(r.resolve("local").is_some());
    }

    #[test]
    fn resolve_unknown_returns_none() {
        let mut r = BackendRegistry::with_builtins();
        assert!(r.resolve("definitely-not-a-backend").is_none());
    }

    #[test]
    fn github_workflow_capabilities_include_approvals_and_matrices() {
        let mut r = BackendRegistry::with_builtins();
        let b = r.resolve("github").unwrap();
        assert!(b.workflow_capabilities().contains(WorkflowCapabilities::APPROVALS));
        assert!(b.workflow_capabilities().contains(WorkflowCapabilities::MATRICES));
        assert!(b.workflow_capabilities().contains(WorkflowCapabilities::RUNNER_SELECTION));
    }

    #[test]
    fn local_workflow_capabilities_include_approvals() {
        let mut r = BackendRegistry::with_builtins();
        let b = r.resolve("local").unwrap();
        assert!(b.workflow_capabilities().contains(WorkflowCapabilities::APPROVALS));
        assert!(!b.workflow_capabilities().contains(WorkflowCapabilities::MATRICES));
    }

    #[test]
    fn inventory_with_mount_actor_derives_mounts_capability() {
        use crate::backends::{BackendCapabilities, derive_capability_profile_from_inventory};
        use crate::ir::{Actor, Inventory};
        let inv = Inventory {
            id: "local".into(),
            actors: vec![Actor {
                id: "local-container".into(),
                labels: vec!["local".into()],
                capabilities: vec!["cache_mount_access".into(), "docker".into()],
                resources: None,
            }],
            placements: vec![crate::ir::InventoryPlacement {
                id: "workspace".into(),
                kind: "volume".into(),
                access_modes: vec!["mount_rw".into(), "same_host".into()],
                accessible_by: vec![],
            }],
            implementations: vec![],
        };
        let profile = derive_capability_profile_from_inventory(Some(&inv));
        assert!(profile.contains(BackendCapabilities::MOUNTS));
        assert!(profile.contains(BackendCapabilities::COLOCATION));
    }

    #[test]
    fn inventory_with_persistent_placement_derives_cache_capability() {
        use crate::backends::{BackendCapabilities, derive_capability_profile_from_inventory};
        use crate::ir::{Inventory, InventoryPlacement};
        let inv = Inventory {
            id: "github".into(),
            actors: vec![],
            placements: vec![InventoryPlacement {
                id: "github-cache".into(),
                kind: "cache".into(),
                access_modes: vec!["persistent".into(), "cross_host".into()],
                accessible_by: vec![],
            }],
            implementations: vec![],
        };
        let profile = derive_capability_profile_from_inventory(Some(&inv));
        assert!(profile.contains(BackendCapabilities::CACHE));
        assert!(!profile.contains(BackendCapabilities::MOUNTS));
    }
}

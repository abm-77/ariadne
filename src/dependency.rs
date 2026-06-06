//! The dependency store: a central catalogue of the tools implementations can
//! need, each mapped to the package that provides it and the manager ecosystem.
//! Lowerings only name the tools they need (e.g. "maturin"); they never repeat
//! install commands. Installation itself is expressed as the `package.install`
//! semantic action with per-manager lowerings (pip, cargo, apt, dnf, ...), so a
//! tool's install command is produced by the same lowering machinery as
//! everything else and a system package lowers to apt or dnf as the actor's
//! inventory dictates.

use std::collections::BTreeMap;

/// A tool resolved to the package that provides it. `manager` pins the install
/// implementation (e.g. "pip", "cargo"); `None` means a system package whose
/// manager (apt/dnf/...) is selected from the inventory when lowered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRef {
    pub manager: Option<String>,
    pub package: String,
}

/// The catalogue of known tools. Built-in entries cover common CI tools; an
/// unknown tool is assumed to be a system package of the same name.
pub struct DependencyRegistry {
    tools: BTreeMap<&'static str, (Option<&'static str>, &'static str)>,
}

impl DependencyRegistry {
    pub fn builtin() -> Self {
        let mut tools = BTreeMap::new();
        for (tool, manager, package) in [
            ("maturin", Some("pip"), "maturin"),
            ("uv", Some("pip"), "uv"),
            ("pytest", Some("pip"), "pytest"),
            ("pytest-cov", Some("pip"), "pytest-cov"),
            ("ruff", Some("pip"), "ruff"),
            ("pdoc", Some("pip"), "pdoc"),
            ("mkdocs", Some("pip"), "mkdocs"),
            ("twine", Some("pip"), "twine"),
            ("cargo-llvm-cov", Some("cargo"), "cargo-llvm-cov"),
            ("git", None, "git"),
        ] {
            tools.insert(tool, (manager, package));
        }
        DependencyRegistry { tools }
    }

    /// The package providing a tool; an unknown tool is assumed to be a system
    /// package of the same name (manager chosen at lowering time).
    pub fn resolve(&self, tool: &str) -> PackageRef {
        match self.tools.get(tool) {
            Some((manager, package)) => PackageRef {
                manager: manager.map(str::to_string),
                package: package.to_string(),
            },
            None => PackageRef { manager: None, package: tool.to_string() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_tool_resolves_to_package_and_manager() {
        let r = DependencyRegistry::builtin();
        assert_eq!(r.resolve("maturin"), PackageRef { manager: Some("pip".into()), package: "maturin".into() });
        assert_eq!(r.resolve("cargo-llvm-cov"), PackageRef { manager: Some("cargo".into()), package: "cargo-llvm-cov".into() });
        assert_eq!(r.resolve("git"), PackageRef { manager: None, package: "git".into() });
    }

    #[test]
    fn unknown_tool_is_a_system_package_of_the_same_name() {
        let r = DependencyRegistry::builtin();
        assert_eq!(r.resolve("ripgrep"), PackageRef { manager: None, package: "ripgrep".into() });
    }
}

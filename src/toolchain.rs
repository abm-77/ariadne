//! Toolchains: the language runtimes a job needs (rust, python, node, ...).
//! Declared in the inventory via `.use("python", version=...)` /
//! `.use("rust", channel=...)`, they are provisioned per job by lowering to the
//! backend's native setup steps (e.g. actions/setup-python). This is the base
//! layer beneath the `dependency` store, which installs the extra tools on top.
//!
//! The mapping here is backend-agnostic: it says which toolchain an
//! implementation belongs to. How a toolchain is set up is the backend's job.

/// The toolchain an implementation runs on, or None for ones that need no
/// language runtime (system tools, containers).
pub fn toolchain_for_impl(implementation: &str) -> Option<&'static str> {
    match implementation {
        "cargo" => Some("rust"),
        "pytest" | "maturin" | "ruff" | "pdoc" | "mkdocs" | "uv" | "pip" | "twine" | "python" => {
            Some("python")
        }
        "npm" | "node" => Some("node"),
        "go" => Some("go"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_implementations_to_toolchains() {
        assert_eq!(toolchain_for_impl("cargo"), Some("rust"));
        assert_eq!(toolchain_for_impl("maturin"), Some("python"));
        assert_eq!(toolchain_for_impl("pytest"), Some("python"));
        assert_eq!(toolchain_for_impl("docker"), None);
    }
}

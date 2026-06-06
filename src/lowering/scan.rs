use super::{arg_str, def, local, Registry};

/// Supply-chain scanning lowerings (SBOM, vulnerability).
pub fn register(r: &mut Registry) {
    r.register(def("scan.sbom.syft", "scan.sbom", "syft", |a| {
        let image = arg_str(a, "image").unwrap_or_default();
        let fmt = arg_str(a, "format").unwrap_or_else(|| "spdx-json".into());
        let out = arg_str(a, "output").unwrap_or_else(|| "sbom.spdx.json".into());
        local(vec!["syft".into(), image, "-o".into(), fmt, ">".into(), out])
    }));
    r.register(def("scan.vulnerability.grype", "scan.vulnerability", "grype", |a| {
        local(vec!["grype".into(), arg_str(a, "image").unwrap_or_default()])
    }));
    r.register(def("scan.vulnerability.trivy", "scan.vulnerability", "trivy", |a| {
        local(vec!["trivy".into(), "image".into(), arg_str(a, "image").unwrap_or_default()])
    }));
}

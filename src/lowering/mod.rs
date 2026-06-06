//! Extensible lowering model.
//!
//! A semantic action (e.g. `build.python_wheel`) does not name a tool. A
//! `LoweringDef` teaches Ariadne how one implementation (e.g. `maturin`)
//! realizes one action. Lowerings are data records held in a `Registry`;
//! built-in packs (one module per high-level class) register defaults and
//! callers may register their own. The inventory declares which implementations
//! are available; selection finds the compatible lowerings and picks one by
//! preference, then cost, then stability.
//!
//! Lowerings are never user-facing: workflow authors choose semantic actions,
//! inventory authors choose implementations, lowering authors teach Ariadne how
//! an implementation realizes an action.
//!
//! This is *implementation selection* (plan-time, backend-agnostic). It is a
//! distinct layer from *instruction selection* (emit-time), which lives in each
//! backend's `instructions.rs` Catalogue and renders the resulting portable
//! `LogicalOp`s as native steps.

mod build;
mod coverage;
mod docs;
mod fmt;
mod forge;
mod package;
mod scan;
mod scm;
mod sign;
mod test;

use crate::diagnostics::{DiagCode, Diagnostic};
use crate::ir::Inventory;
use crate::select::{self, Candidate, Capability, Stability};
use serde_json::Value;
use std::collections::BTreeMap;

pub type Args = BTreeMap<String, Value>;

/// A structured, inspectable lowering body composed from typed execution
/// primitives. Ariadne does not introduce a lowering DSL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoweringBody {
    /// Run a script directly on the actor.
    LocalExec { script: String },
    /// Run a script inside a container image.
    ContainerExec { image: String, script: String },
    /// An ordered sequence of bodies.
    StepSequence(Vec<LoweringBody>),
    /// A portable semantic instruction a backend may upgrade to a native step
    /// (e.g. a GitHub `uses:` action) at emit time via its catalogue. `fallback`
    /// is the shell command any backend can run when it has no native mapping,
    /// keeping the plan portable and a legal plan always available. Choosing the
    /// native step is an emit-time, backend-aware decision, never a plan-time one.
    /// `scm.checkout` is just an instance of this: fallback `git checkout .`,
    /// upgraded to `actions/checkout@v4` on GitHub.
    Native {
        id: String,
        args: BTreeMap<String, String>,
        fallback: String,
    },
}

/// How an implementation realizes a semantic action. `build` constructs the
/// structured body from the call's args, so conditional shaping stays in code
/// while the produced body stays structured.
#[derive(Clone)]
pub struct LoweringDef {
    pub id: &'static str,
    pub action: &'static str,
    pub implementation: &'static str,
    /// Hard actor-capability gates (e.g. "docker"). Empty for most built-ins.
    pub requirements: Vec<Capability>,
    /// Names of tools this implementation needs (e.g. "maturin"). How each
    /// installs is defined once in the `dependency` store, not here. Assumed
    /// provided unless the workflow opts into installing dependencies.
    pub dependencies: Vec<&'static str>,
    pub stability: Stability,
    pub build: fn(&Args) -> LoweringBody,
}

impl LoweringDef {
    /// Declare the tools this implementation needs, by name.
    pub(crate) fn with_deps(mut self, tools: &[&'static str]) -> Self {
        self.dependencies = tools.to_vec();
        self
    }
}

impl Candidate for LoweringDef {
    fn key(&self) -> &str {
        self.action
    }
    fn requires(&self) -> &[Capability] {
        &self.requirements
    }
    fn stability(&self) -> Stability {
        self.stability
    }
}

/// The concrete realization handed to the planner. Maps onto the existing
/// LogicalOp vocabulary so backends need no knowledge of semantic actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Realization {
    Shell(String),
    /// A portable semantic instruction with a shell fallback; backends decide
    /// emit-time whether to upgrade it to a native step.
    Native {
        id: String,
        args: BTreeMap<String, String>,
        fallback: String,
    },
}

/// The outcome of selection: the chosen lowering, what it realizes to, and a
/// human-readable reason for explainability.
#[derive(Debug, Clone)]
pub struct Selection {
    pub lowering_id: String,
    pub implementation: String,
    pub realization: Realization,
    pub reason: String,
    /// Names of tools the chosen implementation needs (resolved against the
    /// `dependency` store for install-on-start emission).
    pub dependencies: Vec<String>,
}

/// A collection of lowering definitions, keyed by semantic action. This is the
/// shared `select::Registry` specialized to `LoweringDef`; the methods below add
/// the lowering-specific `builtin` set and `select` policy. Built-in packs
/// register defaults; callers (and, in future, distributed lowering packages)
/// may register more.
pub type Registry = crate::select::Registry<LoweringDef>;

impl Registry {
    /// The registry with all built-in lowering packs registered.
    pub fn builtin() -> Self {
        let mut r = Self::new();
        scm::register(&mut r);
        build::register(&mut r);
        test::register(&mut r);
        fmt::register(&mut r);
        docs::register(&mut r);
        coverage::register(&mut r);
        scan::register(&mut r);
        sign::register(&mut r);
        package::register(&mut r);
        forge::register(&mut r);
        r
    }

    /// Select a lowering for a semantic action, using inventory-based ranking
    /// (deny/prefer/available/default). See `select_using` for the per-call pin.
    pub fn select(
        &self,
        action: &str,
        args: &Args,
        inv: Option<&Inventory>,
        actor_caps: &[Capability],
    ) -> Result<Selection, Diagnostic> {
        self.select_using(action, args, inv, actor_caps, None, &[])
    }

    /// Like `select`, but `using` optionally pins the implementation for this
    /// call (e.g. "pytest"), overriding inventory ranking. Honors deny, validates
    /// requirements against the actor's capabilities, and otherwise ranks
    /// preferred first, then available, then undeclared defaults (ties: stability
    /// then registration order). A correct plan always exists when the inventory
    /// is silent and no pin is given.
    pub fn select_using(
        &self,
        action: &str,
        args: &Args,
        inv: Option<&Inventory>,
        actor_caps: &[Capability],
        using: Option<&str>,
        prefer: &[String],
    ) -> Result<Selection, Diagnostic> {
        let cands: Vec<&LoweringDef> = self.candidates(action).collect();
        if cands.is_empty() {
            return Err(Diagnostic::error(
                DiagCode::UnknownSemanticOp,
                format!("no lowering registered for semantic action '{action}'"),
            ));
        }

        let denied = |impl_id: &str| {
            inv.is_some_and(|i| i.implementations.iter().any(|m| m.id == impl_id && m.deny))
        };

        // A call may pin its implementation (`using=`), overriding inventory
        // ranking. The pinned impl must exist for the action, not be denied, and
        // meet the actor's capabilities.
        if let Some(want) = using {
            let def = cands.iter().copied().find(|d| d.implementation == want).ok_or_else(|| {
                Diagnostic::error(
                    DiagCode::NoCompatibleImplementation,
                    format!("call pins implementation '{want}' for '{action}', which has no such lowering"),
                )
            })?;
            if denied(def.implementation) || !select::requirements_met(def, actor_caps) {
                return Err(Diagnostic::error(
                    DiagCode::NoCompatibleImplementation,
                    format!(
                        "pinned implementation '{want}' for '{action}' is denied or has unmet requirements"
                    ),
                ));
            }
            return Ok(Selection {
                lowering_id: def.id.to_string(),
                implementation: def.implementation.to_string(),
                realization: realize((def.build)(args)),
                reason: format!("call pins implementation '{want}' for {action}"),
                dependencies: def.dependencies.iter().map(|s| s.to_string()).collect(),
            });
        }
        let preferred = |impl_id: &str| {
            inv.is_some_and(|i| {
                i.implementations
                    .iter()
                    .any(|m| m.id == impl_id && m.prefer)
            })
        };
        let available = |impl_id: &str| {
            inv.is_some_and(|i| i.implementations.iter().any(|m| m.id == impl_id && !m.deny))
        };
        // Inventory preference is this layer's soft ranking: prefer (0) beats
        // declared-available (1) beats undeclared default (2). It never excludes
        // (silent inventory still yields a default) — deny does the excluding.
        let priority = |d: &LoweringDef| {
            if preferred(d.implementation) {
                0
            } else if available(d.implementation) {
                1
            } else {
                2
            }
        };

        // Deny is the lowering-specific exclusion; the actor-capability gate is
        // hard. Materialize the eligible candidates once.
        let eligible: Vec<&LoweringDef> = cands
            .iter()
            .copied()
            .filter(|d| !denied(d.implementation))
            .filter(|d| select::requirements_met(*d, actor_caps))
            .collect();
        if eligible.is_empty() {
            return Err(Diagnostic::error(
                DiagCode::NoCompatibleImplementation,
                format!(
                    "no compatible lowering for '{action}': every candidate is denied or has unmet requirements"
                ),
            ));
        }

        // A scoped `impl`/`impls` binding softly prefers these implementations,
        // in order, where one is an eligible candidate for this action.
        for want in prefer {
            if let Some(def) = eligible
                .iter()
                .copied()
                .find(|d| d.implementation == want.as_str())
            {
                return Ok(Selection {
                    lowering_id: def.id.to_string(),
                    implementation: def.implementation.to_string(),
                    realization: realize((def.build)(args)),
                    reason: format!("scope prefers '{want}' for {action}"),
                    dependencies: def.dependencies.iter().map(|s| s.to_string()).collect(),
                });
            }
        }

        // Otherwise rank by inventory preference, then stability, then order.
        let def = select::resolve(eligible.iter().copied(), actor_caps, priority)
            .expect("eligible is non-empty");
        let why = match priority(def) {
            0 => format!("inventory prefers implementation '{}'", def.implementation),
            1 => format!(
                "implementation '{}' available in inventory",
                def.implementation
            ),
            _ => format!(
                "default implementation '{}' (inventory silent)",
                def.implementation
            ),
        };

        Ok(Selection {
            lowering_id: def.id.to_string(),
            implementation: def.implementation.to_string(),
            realization: realize((def.build)(args)),
            reason: format!("selected lowering '{}': {why}", def.id),
            dependencies: def.dependencies.iter().map(|s| s.to_string()).collect(),
        })
    }
}

/// Flatten a structured body into the planner's realization vocabulary. Exec
/// bodies become shell scripts (the container image is informational at this
/// layer, matching how the planner treats container implementations today).
fn realize(body: LoweringBody) -> Realization {
    match body {
        LoweringBody::LocalExec { script } => Realization::Shell(script),
        LoweringBody::ContainerExec { script, .. } => Realization::Shell(script),
        LoweringBody::Native { id, args, fallback } => Realization::Native { id, args, fallback },
        LoweringBody::StepSequence(bodies) => {
            let mut lines = Vec::new();
            for b in bodies {
                match realize(b) {
                    Realization::Shell(s) => lines.push(s),
                    Realization::Native { fallback, .. } => lines.push(fallback),
                }
            }
            Realization::Shell(lines.join("\n"))
        }
    }
}

pub(crate) fn arg_str(args: &Args, key: &str) -> Option<String> {
    args.get(key).and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    })
}

pub(crate) fn arg_flag(args: &Args, key: &str) -> bool {
    matches!(args.get(key), Some(Value::Bool(true)))
}

pub(crate) fn arg_list(args: &Args, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => vec![],
    }
}

pub(crate) fn local(parts: Vec<String>) -> LoweringBody {
    LoweringBody::LocalExec {
        script: parts.join(" "),
    }
}

/// Run inside a container image. No built-in pack uses this today; it remains
/// for lowering authors (and is exercised by tests).
#[cfg(test)]
pub(crate) fn container(image: &str, parts: Vec<String>) -> LoweringBody {
    LoweringBody::ContainerExec {
        image: image.to_string(),
        script: parts.join(" "),
    }
}

/// Build a stable lowering def with no requirements. Packs use this for the
/// common case; set fields directly for requirements or non-default stability.
/// Default preference among same-class candidates is registration order, so
/// register the preferred implementation first.
pub(crate) fn def(
    id: &'static str,
    action: &'static str,
    implementation: &'static str,
    build: fn(&Args) -> LoweringBody,
) -> LoweringDef {
    LoweringDef {
        id,
        action,
        implementation,
        requirements: vec![],
        dependencies: vec![],
        stability: Stability::Stable,
        build,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::InventoryImpl;
    use serde_json::json;
    use ustr::Ustr;

    fn inv(impls: Vec<InventoryImpl>) -> Inventory {
        Inventory {
            id: "t".into(),
            actors: vec![],
            placements: vec![],
            implementations: impls,
        }
    }
    fn used(id: &str) -> InventoryImpl {
        InventoryImpl {
            id: Ustr::from(id),
            version: None,
            prefer: false,
            deny: false,
        }
    }
    fn pref(id: &str) -> InventoryImpl {
        InventoryImpl {
            id: Ustr::from(id),
            version: None,
            prefer: true,
            deny: false,
        }
    }
    fn denied(id: &str) -> InventoryImpl {
        InventoryImpl {
            id: Ustr::from(id),
            version: None,
            prefer: false,
            deny: true,
        }
    }

    fn reg() -> Registry {
        Registry::builtin()
    }

    #[test]
    fn package_install_lowers_per_manager() {
        // Each manager produces its ecosystem's install command. The planner
        // pins the manager (language packages directly, system packages via the
        // actor's system manager), so we exercise the pinned lowerings here.
        let mut a = Args::new();
        a.insert("package".into(), json!("maturin"));
        let pip = reg()
            .select_using("package.install", &a, None, &[], Some("pip"), &[])
            .unwrap();
        assert!(matches!(pip.realization, Realization::Shell(s) if s == "pip install maturin"));

        let mut g = Args::new();
        g.insert("package".into(), json!("git"));
        let apt = reg()
            .select_using("package.install", &g, None, &[], Some("apt"), &[])
            .unwrap();
        assert!(
            matches!(apt.realization, Realization::Shell(s) if s == "sudo apt-get install -y git")
        );
        let dnf = reg()
            .select_using("package.install", &g, None, &[], Some("dnf"), &[])
            .unwrap();
        assert!(matches!(dnf.realization, Realization::Shell(s) if s == "sudo dnf install -y git"));

        let mut c = Args::new();
        c.insert("package".into(), json!("cargo-llvm-cov"));
        let cargo = reg()
            .select_using("package.install", &c, None, &[], Some("cargo"), &[])
            .unwrap();
        assert!(
            matches!(cargo.realization, Realization::Shell(s) if s == "cargo install cargo-llvm-cov")
        );
    }

    #[test]
    fn unknown_action_errors() {
        let err = reg()
            .select("nope.nope", &Args::new(), None, &[])
            .unwrap_err();
        assert_eq!(err.code, DiagCode::UnknownSemanticOp);
    }

    #[test]
    fn checkout_realizes_to_native_with_git_fallback() {
        let sel = reg()
            .select("scm.checkout", &Args::new(), None, &[])
            .unwrap();
        match &sel.realization {
            Realization::Native { id, fallback, .. } => {
                assert_eq!(id, "scm.checkout");
                assert_eq!(fallback, "git checkout .");
            }
            other => panic!("expected Native, got {other:?}"),
        }
        assert_eq!(sel.implementation, "git");
        assert_eq!(sel.lowering_id, "scm.checkout.git");
    }

    #[test]
    fn silent_inventory_uses_cheapest_default() {
        let sel = reg()
            .select("build.python_wheel", &Args::new(), None, &[])
            .unwrap();
        assert_eq!(sel.implementation, "maturin");
        assert!(sel.reason.contains("silent"));
    }

    #[test]
    fn available_beats_default() {
        let i = inv(vec![used("uv")]);
        let sel = reg()
            .select("build.python_wheel", &Args::new(), Some(&i), &[])
            .unwrap();
        assert_eq!(sel.implementation, "uv");
        assert_eq!(sel.lowering_id, "build.python_wheel.uv");
    }

    #[test]
    fn preferred_beats_available() {
        let i = inv(vec![used("maturin"), pref("uv")]);
        let sel = reg()
            .select("build.python_wheel", &Args::new(), Some(&i), &[])
            .unwrap();
        assert_eq!(sel.implementation, "uv");
        assert!(sel.reason.contains("prefers"));
    }

    #[test]
    fn publish_realizes_to_native_with_shell_fallback() {
        let mut a = Args::new();
        a.insert("dist".into(), json!("dist/*.whl"));
        let sel = reg().select("package.publish", &a, None, &[]).unwrap();
        match sel.realization {
            Realization::Native { id, fallback, .. } => {
                assert_eq!(id, "package.publish");
                assert_eq!(fallback, "twine upload dist/*.whl");
            }
            other => panic!("expected Native, got {other:?}"),
        }
    }

    #[test]
    fn deny_excludes_and_falls_through() {
        let i = inv(vec![denied("maturin")]);
        let sel = reg()
            .select("build.python_wheel", &Args::new(), Some(&i), &[])
            .unwrap();
        assert_ne!(sel.implementation, "maturin");
    }

    #[test]
    fn all_denied_errors() {
        let i = inv(vec![denied("git")]);
        let err = reg()
            .select("scm.checkout", &Args::new(), Some(&i), &[])
            .unwrap_err();
        assert_eq!(err.code, DiagCode::NoCompatibleImplementation);
    }

    #[test]
    fn unmet_requirement_excludes_candidate() {
        let mut r = Registry::new();
        r.register(LoweringDef {
            id: "build.container_image.docker",
            action: "build.container_image",
            implementation: "docker",
            requirements: vec![Capability::new("docker")],
            dependencies: vec![],
            stability: Stability::Stable,
            build: |_| local(vec!["docker".into(), "build".into()]),
        });
        let err = r
            .select("build.container_image", &Args::new(), None, &[])
            .unwrap_err();
        assert_eq!(err.code, DiagCode::NoCompatibleImplementation);
        let sel = r
            .select(
                "build.container_image",
                &Args::new(),
                None,
                &[Capability::new("docker")],
            )
            .unwrap();
        assert_eq!(sel.implementation, "docker");
    }

    #[test]
    fn cargo_build_renders_flags() {
        let mut a = Args::new();
        a.insert("release".into(), json!(true));
        a.insert("package".into(), json!("loom"));
        let sel = reg()
            .select("build.binary", &a, Some(&inv(vec![used("cargo")])), &[])
            .unwrap();
        assert_eq!(
            sel.realization,
            Realization::Shell("cargo build --release --package loom".into())
        );
    }

    #[test]
    fn maturin_renders_manifest_and_out() {
        let mut a = Args::new();
        a.insert("release".into(), json!(true));
        a.insert("manifest".into(), json!("crates/ariadne-py/Cargo.toml"));
        a.insert("out".into(), json!("dist"));
        let sel = reg().select("build.python_wheel", &a, None, &[]).unwrap();
        assert_eq!(
            sel.realization,
            Realization::Shell(
                "maturin build --release --manifest-path crates/ariadne-py/Cargo.toml --out dist"
                    .into()
            )
        );
    }

    #[test]
    fn test_unit_lowers_to_cargo_or_pytest_by_inventory() {
        let mut a = Args::new();
        a.insert("args".into(), json!(["--workspace"]));
        let cargo = reg()
            .select("test.unit", &a, Some(&inv(vec![used("cargo")])), &[])
            .unwrap();
        assert_eq!(
            cargo.realization,
            Realization::Shell("cargo test --workspace".into())
        );

        let mut b = Args::new();
        b.insert("paths".into(), json!(["tests/"]));
        b.insert("args".into(), json!(["-v"]));
        let py = reg()
            .select("test.unit", &b, Some(&inv(vec![used("pytest")])), &[])
            .unwrap();
        assert_eq!(
            py.realization,
            Realization::Shell("pytest tests/ -v".into())
        );
    }

    #[test]
    fn using_pins_implementation_in_a_mixed_inventory() {
        // Inventory offers both runners; the pin disambiguates per call.
        let i = inv(vec![used("cargo"), used("pytest")]);
        let mut a = Args::new();
        a.insert("args".into(), json!(["--workspace"]));
        let cargo = reg()
            .select_using("test.unit", &a, Some(&i), &[], Some("cargo"), &[])
            .unwrap();
        assert_eq!(
            cargo.realization,
            Realization::Shell("cargo test --workspace".into())
        );
        assert!(cargo.reason.contains("pins implementation 'cargo'"));

        let mut b = Args::new();
        b.insert("paths".into(), json!(["tests/"]));
        let py = reg()
            .select_using("test.unit", &b, Some(&i), &[], Some("pytest"), &[])
            .unwrap();
        assert_eq!(py.realization, Realization::Shell("pytest tests/".into()));
    }

    #[test]
    fn using_unknown_implementation_errors() {
        let err = reg()
            .select_using("test.unit", &Args::new(), None, &[], Some("nose"), &[])
            .unwrap_err();
        assert_eq!(err.code, DiagCode::NoCompatibleImplementation);
    }

    #[test]
    fn using_denied_implementation_errors() {
        let i = inv(vec![denied("pytest")]);
        let err = reg()
            .select_using(
                "test.unit",
                &Args::new(),
                Some(&i),
                &[],
                Some("pytest"),
                &[],
            )
            .unwrap_err();
        assert_eq!(err.code, DiagCode::NoCompatibleImplementation);
    }

    #[test]
    fn user_registered_lowering_is_selected_when_preferred() {
        let mut r = Registry::builtin();
        r.register(LoweringDef {
            id: "build.python_wheel.company",
            action: "build.python_wheel",
            implementation: "company-wheel-builder",
            requirements: vec![],
            dependencies: vec![],
            stability: Stability::Stable,
            build: |a| {
                container(
                    "registry.company.com/build/python-wheel:latest",
                    vec![
                        "company-build-wheel".into(),
                        "--package".into(),
                        arg_str(a, "package").unwrap_or_default(),
                    ],
                )
            },
        });
        let i = inv(vec![pref("company-wheel-builder")]);
        let mut a = Args::new();
        a.insert("package".into(), json!("ariadne-bindings"));
        let sel = r.select("build.python_wheel", &a, Some(&i), &[]).unwrap();
        assert_eq!(sel.lowering_id, "build.python_wheel.company");
        assert_eq!(
            sel.realization,
            Realization::Shell("company-build-wheel --package ariadne-bindings".into())
        );
    }
}

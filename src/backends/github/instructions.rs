use crate::backends::{
    BackendKind, Bindings, Capability, Catalogue, CostHint, Instruction, InstructionId,
    OpMatcher, Stability,
};
use serde_json::json;

pub fn catalogue() -> Catalogue {
    Catalogue::from_items(vec![
        Instruction {
            id: InstructionId("github.checkout.default".into()),
            backend: BackendKind::Github,
            provides: vec![Capability::new("repo.checkout")],
            requires: vec![Capability::new("github.action_calls.uses")],
            matcher: OpMatcher::for_op("CheckoutRepo"),
            cost: CostHint { fixed: 5, per_mb: 0 },
            stability: Stability::Stable,
            implementation: json!({
                "kind": "github.uses",
                "ref": "actions/checkout@v4",
                "name": "Checkout repository"
            }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("github.shell.run".into()),
            backend: BackendKind::Github,
            provides: vec![Capability::new("process.exec")],
            requires: vec![],
            matcher: OpMatcher::for_op("RunShell"),
            cost: CostHint { fixed: 1, per_mb: 0 },
            stability: Stability::Stable,
            implementation: json!({ "kind": "github.run" }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("github.artifact.upload".into()),
            backend: BackendKind::Github,
            provides: vec![Capability::new("artifact.upload")],
            requires: vec![Capability::new("github.action_calls.uses")],
            matcher: OpMatcher::for_op("UploadArtifact"),
            cost: CostHint { fixed: 5, per_mb: 1 },
            stability: Stability::Stable,
            implementation: json!({
                "kind": "github.uses",
                "ref": "actions/upload-artifact@v4"
            }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("github.artifact.download".into()),
            backend: BackendKind::Github,
            provides: vec![Capability::new("artifact.download")],
            requires: vec![Capability::new("github.action_calls.uses")],
            matcher: OpMatcher::for_op("DownloadArtifact"),
            cost: CostHint { fixed: 5, per_mb: 1 },
            stability: Stability::Stable,
            implementation: json!({
                "kind": "github.uses",
                "ref": "actions/download-artifact@v4"
            }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("github.cache.restore".into()),
            backend: BackendKind::Github,
            provides: vec![Capability::new("cache.restore")],
            requires: vec![Capability::new("github.action_calls.uses")],
            matcher: OpMatcher::for_op("RestoreCache"),
            cost: CostHint { fixed: 5, per_mb: 0 },
            stability: Stability::Stable,
            implementation: json!({
                "kind": "github.uses",
                "ref": "actions/cache@v4",
                "cache_action": "restore"
            }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("github.cache.save".into()),
            backend: BackendKind::Github,
            provides: vec![Capability::new("cache.save")],
            requires: vec![Capability::new("github.action_calls.uses")],
            matcher: OpMatcher::for_op("SaveCache"),
            cost: CostHint { fixed: 5, per_mb: 0 },
            stability: Stability::Stable,
            implementation: json!({
                "kind": "github.uses",
                "ref": "actions/cache/save@v4",
                "cache_action": "save"
            }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("github.approval.gate".into()),
            backend: BackendKind::Github,
            provides: vec![Capability::new("approval.gate")],
            requires: vec![],
            matcher: OpMatcher::for_op("RequestApproval"),
            cost: CostHint { fixed: 0, per_mb: 0 },
            stability: Stability::Stable,
            implementation: json!({ "kind": "github.environment" }),
            bind: Bindings::default(),
        },
        // Native checkout upgrade: emit actions/checkout@v4 instead of the
        // `git checkout` shell fallback. Always available on GitHub (gated only
        // on the static uses capability), so it is the idiomatic default.
        Instruction {
            id: InstructionId("github.checkout.native".into()),
            backend: BackendKind::Github,
            provides: vec![Capability::new("repo.checkout")],
            requires: vec![Capability::new("github.action_calls.uses")],
            matcher: {
                let mut m = OpMatcher::for_op("Native");
                m.extra.insert("native_id".into(), "scm.checkout".into());
                m
            },
            cost: CostHint { fixed: 3, per_mb: 0 },
            stability: Stability::Stable,
            implementation: json!({
                "kind": "github.uses",
                "ref": "actions/checkout@v4",
                "name": "Checkout repository"
            }),
            bind: Bindings::default(),
        },
        // Native publish upgrade: emit the pypa publishing Action instead of the
        // shell fallback when the inventory declares it available. Gated on the
        // inventory-derived capability so it never fires unless permitted.
        Instruction {
            id: InstructionId("github.publish.pypa".into()),
            backend: BackendKind::Github,
            provides: vec![Capability::new("package.publish")],
            requires: vec![Capability::new("impl.pypa-publish-action")],
            matcher: {
                let mut m = OpMatcher::for_op("Native");
                m.extra.insert("native_id".into(), "package.publish".into());
                m
            },
            cost: CostHint { fixed: 4, per_mb: 0 },
            stability: Stability::Stable,
            implementation: json!({
                "kind": "github.uses",
                "ref": "pypa/gh-action-pypi-publish@release/v1"
            }),
            bind: Bindings::default(),
        },
        // Portable fallback for any Native op: run its shell command. Higher cost
        // so a native upgrade wins when its capability is present.
        Instruction {
            id: InstructionId("github.native.fallback".into()),
            backend: BackendKind::Github,
            provides: vec![],
            requires: vec![],
            matcher: OpMatcher::for_op("Native"),
            cost: CostHint { fixed: 10, per_mb: 0 },
            stability: Stability::Stable,
            implementation: json!({ "kind": "github.run.native" }),
            bind: Bindings::default(),
        },
    ])
}

pub fn capabilities() -> Vec<Capability> {
    vec![
        Capability::new("github.action_calls.uses"),
        Capability::new("process.exec"),
    ]
}

use crate::backends::{
    BackendKind, Bindings, Capability, Catalogue, CostHint, Instruction, InstructionId,
    OpMatcher, Stability,
};
use serde_json::json;

pub fn catalogue() -> Catalogue {
    Catalogue::new(vec![
        Instruction {
            id: InstructionId("github.checkout.default".into()),
            backend: BackendKind::Github,
            provides: vec![Capability::new("repo.checkout")],
            requires: vec![Capability::new("github.actions.uses")],
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
            requires: vec![Capability::new("github.actions.uses")],
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
            requires: vec![Capability::new("github.actions.uses")],
            matcher: OpMatcher::for_op("DownloadArtifact"),
            cost: CostHint { fixed: 5, per_mb: 1 },
            stability: Stability::Stable,
            implementation: json!({
                "kind": "github.uses",
                "ref": "actions/download-artifact@v4"
            }),
            bind: Bindings::default(),
        },
    ])
}

pub fn capabilities() -> Vec<Capability> {
    vec![
        Capability::new("github.actions.uses"),
        Capability::new("process.exec"),
    ]
}

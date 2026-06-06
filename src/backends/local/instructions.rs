use crate::backends::{
    BackendKind, Bindings, Capability, Catalogue, CostHint, Instruction, InstructionId, OpMatcher,
    Stability,
};
use serde_json::json;

pub fn catalogue() -> Catalogue {
    Catalogue::from_items(vec![
        Instruction {
            id: InstructionId("local.checkout.git".into()),
            backend: BackendKind::Local,
            provides: vec![Capability::new("repo.checkout")],
            requires: vec![Capability::new("process.exec")],
            matcher: OpMatcher::for_op("CheckoutRepo"),
            cost: CostHint {
                fixed: 3,
                per_mb: 0,
            },
            stability: Stability::Stable,
            implementation: json!({ "kind": "process.exec", "argv": ["git", "checkout", "."] }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("local.shell.run".into()),
            backend: BackendKind::Local,
            provides: vec![Capability::new("process.exec")],
            requires: vec![Capability::new("process.exec")],
            matcher: OpMatcher::for_op("RunShell"),
            cost: CostHint {
                fixed: 1,
                per_mb: 0,
            },
            stability: Stability::Stable,
            implementation: json!({ "kind": "process.exec" }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("local.artifact.upload".into()),
            backend: BackendKind::Local,
            provides: vec![Capability::new("artifact.upload")],
            requires: vec![Capability::new("process.exec")],
            matcher: OpMatcher::for_op("UploadArtifact"),
            cost: CostHint {
                fixed: 1,
                per_mb: 0,
            },
            stability: Stability::Stable,
            implementation: json!({ "kind": "local.copy" }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("local.artifact.download".into()),
            backend: BackendKind::Local,
            provides: vec![Capability::new("artifact.download")],
            requires: vec![Capability::new("process.exec")],
            matcher: OpMatcher::for_op("DownloadArtifact"),
            cost: CostHint {
                fixed: 1,
                per_mb: 0,
            },
            stability: Stability::Stable,
            implementation: json!({ "kind": "local.noop" }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("local.artifact.mount".into()),
            backend: BackendKind::Local,
            provides: vec![Capability::new("artifact.mount")],
            requires: vec![Capability::new("process.exec")],
            matcher: OpMatcher::for_op("TransferArtifact"),
            cost: CostHint {
                fixed: 0,
                per_mb: 0,
            },
            stability: Stability::Stable,
            implementation: json!({ "kind": "local.noop" }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("local.cache.restore".into()),
            backend: BackendKind::Local,
            provides: vec![Capability::new("cache.restore")],
            requires: vec![Capability::new("process.exec")],
            matcher: OpMatcher::for_op("RestoreCache"),
            cost: CostHint {
                fixed: 1,
                per_mb: 0,
            },
            stability: Stability::Stable,
            implementation: json!({ "kind": "local.cache", "action": "restore" }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("local.cache.save".into()),
            backend: BackendKind::Local,
            provides: vec![Capability::new("cache.save")],
            requires: vec![Capability::new("process.exec")],
            matcher: OpMatcher::for_op("SaveCache"),
            cost: CostHint {
                fixed: 1,
                per_mb: 0,
            },
            stability: Stability::Stable,
            implementation: json!({ "kind": "local.cache", "action": "save" }),
            bind: Bindings::default(),
        },
        Instruction {
            id: InstructionId("local.approval.gate".into()),
            backend: BackendKind::Local,
            provides: vec![Capability::new("approval.gate")],
            requires: vec![Capability::new("process.exec")],
            matcher: OpMatcher::for_op("RequestApproval"),
            cost: CostHint {
                fixed: 0,
                per_mb: 0,
            },
            stability: Stability::Stable,
            implementation: json!({ "kind": "local.prompt" }),
            bind: Bindings::default(),
        },
        // The local backend has no native steps; a Native op always runs its
        // portable shell fallback.
        Instruction {
            id: InstructionId("local.native.fallback".into()),
            backend: BackendKind::Local,
            provides: vec![],
            requires: vec![Capability::new("process.exec")],
            matcher: OpMatcher::for_op("Native"),
            cost: CostHint {
                fixed: 1,
                per_mb: 0,
            },
            stability: Stability::Stable,
            implementation: json!({ "kind": "local.native" }),
            bind: Bindings::default(),
        },
    ])
}

pub fn capabilities() -> Vec<Capability> {
    vec![Capability::new("process.exec")]
}

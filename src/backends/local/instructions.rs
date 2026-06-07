use crate::backends::{
    BackendKind, Bindings, Capability, Catalogue, CostHint, Instruction, InstructionId, OpMatcher,
    Stability,
};
use serde_json::json;

pub fn catalogue() -> Catalogue {
    Catalogue::from_items(vec![
        Instruction {
            id: InstructionId("local.shell.run".into()),
            backend: BackendKind::Local,
            provides: vec![Capability::new("process.exec")],
            requires: vec![Capability::new("process.exec")],
            matcher: OpMatcher::for_action("shell.run"),
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
            matcher: OpMatcher::for_action("ci.artifact.upload"),
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
            matcher: OpMatcher::for_action("ci.artifact.download"),
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
            matcher: OpMatcher::for_action("ci.artifact.transfer"),
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
            matcher: OpMatcher::for_action("ci.cache.restore"),
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
            matcher: OpMatcher::for_action("ci.cache.save"),
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
            matcher: OpMatcher::for_action("ci.approval"),
            cost: CostHint {
                fixed: 0,
                per_mb: 0,
            },
            stability: Stability::Stable,
            implementation: json!({ "kind": "local.prompt" }),
            bind: Bindings::default(),
        },
        // The local backend has no native steps; a semantic op (including
        // scm.checkout) always runs its portable shell fallback.
        Instruction {
            id: InstructionId("local.semantic.fallback".into()),
            backend: BackendKind::Local,
            provides: vec![],
            requires: vec![Capability::new("process.exec")],
            matcher: OpMatcher::any_semantic(),
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

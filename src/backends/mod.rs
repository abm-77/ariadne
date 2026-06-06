pub mod renderers;
pub mod github;
pub mod local;
pub mod registry;
pub mod external;

use crate::diagnostics::Diagnostic;
use crate::planner::{LogicalOp, Plan, AccessMode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use std::collections::{HashMap, HashSet};
use ustr::Ustr;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackendKind {
    Github,
    Local,
    Gitlab,
    Custom(String),
}

// ---------------------------------------------------------------------------
// Capability layers
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct WorkflowCapabilities: u32 {
        const JOBS             = 1 << 0;
        const DEPENDENCIES     = 1 << 1;
        const CONDITIONS       = 1 << 2;
        const MATRICES         = 1 << 3;
        const PERMISSIONS      = 1 << 4;
        const SECRETS          = 1 << 5;
        const APPROVALS        = 1 << 6;
        const RUNNER_SELECTION = 1 << 7;
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct ActorCapabilities: u32 {
        const LINUX                = 1 << 0;
        const WINDOWS              = 1 << 1;
        const MACOS                = 1 << 2;
        const ARM64                = 1 << 3;
        const DOCKER               = 1 << 4;
        const PODMAN               = 1 << 5;
        const GPU                  = 1 << 6;
        const PERSISTENT_WORKSPACE = 1 << 7;
        const MOUNT_ACCESS         = 1 << 8;
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct PlacementCapabilities: u32 {
        const COPY        = 1 << 0;
        const STREAM      = 1 << 1;
        const MOUNT_RO    = 1 << 2;
        const MOUNT_RW    = 1 << 3;
        const PERSISTENT  = 1 << 4;
        const SAME_HOST   = 1 << 5;
        const CROSS_HOST  = 1 << 6;
    }
}

/// An execution resource class the backend can target.
#[derive(Debug, Clone)]
pub struct ActorClass {
    pub id: String,
    pub labels: Vec<String>,
    pub capabilities: ActorCapabilities,
}

/// An artifact storage / access provider the backend supports.
#[derive(Debug, Clone)]
pub struct PlacementProvider {
    pub id: String,
    pub capabilities: PlacementCapabilities,
}

// ---------------------------------------------------------------------------
// Optimizer capability profile (derived from actor + placement declarations)
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    /// High-level flags the optimizer reasons about. Derived from
    /// `ActorClass` and `PlacementProvider` declarations; do not set by hand.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct BackendCapabilities: u32 {
        const MOUNTS     = 1 << 0;
        const COLOCATION = 1 << 1;
        const CACHE      = 1 << 2;
        const STREAM     = 1 << 3;
    }
}

/// Derive the optimizer profile from an Inventory embedded in Thread IR.
/// Returns empty capabilities when no inventory is provided.
pub fn derive_capability_profile_from_inventory(
    inv: Option<&crate::ir::Inventory>,
) -> BackendCapabilities {
    let inv = match inv { Some(i) => i, None => return BackendCapabilities::empty() };
    let actors = inv.actors.iter().map(|a| ActorClass {
        id: a.id.to_string(),
        labels: a.labels.iter().map(|s| s.to_string()).collect(),
        capabilities: actor_caps_from_strings(&a.capabilities),
    }).collect::<Vec<_>>();
    let placements = inv.placements.iter().map(|p| PlacementProvider {
        id: p.id.to_string(),
        capabilities: placement_caps_from_strings(&p.access_modes),
    }).collect::<Vec<_>>();
    derive_capability_profile(&actors, &placements)
}

fn actor_caps_from_strings(v: &[ustr::Ustr]) -> ActorCapabilities {
    let mut caps = ActorCapabilities::empty();
    for s in v {
        match s.as_str() {
            "linux" => caps |= ActorCapabilities::LINUX,
            "windows" => caps |= ActorCapabilities::WINDOWS,
            "macos" => caps |= ActorCapabilities::MACOS,
            "arm64" => caps |= ActorCapabilities::ARM64,
            "docker" => caps |= ActorCapabilities::DOCKER,
            "podman" => caps |= ActorCapabilities::PODMAN,
            "gpu" => caps |= ActorCapabilities::GPU,
            "persistent_workspace" => caps |= ActorCapabilities::PERSISTENT_WORKSPACE,
            "cache_mount_access" | "mount" | "mount_access" => caps |= ActorCapabilities::MOUNT_ACCESS,
            _ => {}
        }
    }
    caps
}

fn placement_caps_from_strings(v: &[ustr::Ustr]) -> PlacementCapabilities {
    let mut caps = PlacementCapabilities::empty();
    for s in v {
        match s.as_str() {
            "copy" => caps |= PlacementCapabilities::COPY,
            "stream" => caps |= PlacementCapabilities::STREAM,
            "mount_ro" => caps |= PlacementCapabilities::MOUNT_RO,
            "mount_rw" => caps |= PlacementCapabilities::MOUNT_RW,
            "persistent" => caps |= PlacementCapabilities::PERSISTENT,
            "same_host" | "same-host" => caps |= PlacementCapabilities::SAME_HOST,
            "cross_host" | "cross-host" => caps |= PlacementCapabilities::CROSS_HOST,
            _ => {}
        }
    }
    caps
}

fn derive_capability_profile(
    actors: &[ActorClass],
    placements: &[PlacementProvider],
) -> BackendCapabilities {
    let mut caps = BackendCapabilities::empty();
    if actors.iter().any(|a| a.capabilities.contains(ActorCapabilities::MOUNT_ACCESS)) {
        caps |= BackendCapabilities::MOUNTS;
    }
    if placements.iter().any(|p| p.capabilities.contains(PlacementCapabilities::SAME_HOST)) {
        caps |= BackendCapabilities::COLOCATION;
    }
    if placements.iter().any(|p| p.capabilities.contains(PlacementCapabilities::PERSISTENT)) {
        caps |= BackendCapabilities::CACHE;
    }
    if placements.iter().any(|p| p.capabilities.contains(PlacementCapabilities::STREAM)) {
        caps |= BackendCapabilities::STREAM;
    }
    caps
}

// ---------------------------------------------------------------------------
// Instruction-selection layer
// ---------------------------------------------------------------------------

pub use crate::select::{Capability, Candidate, Stability};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostHint {
    pub fixed: u32,
    pub per_mb: u32,
}

impl Default for CostHint {
    fn default() -> Self { Self { fixed: 5, per_mb: 0 } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InstructionId(pub Ustr);

impl<S: AsRef<str>> From<S> for InstructionId {
    fn from(s: S) -> Self { Self(Ustr::from(s.as_ref())) }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpMatcher {
    pub op: String,
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

impl OpMatcher {
    pub fn for_op(op: &str) -> Self {
        Self { op: op.into(), extra: HashMap::new() }
    }

    pub fn matches(&self, op: &LogicalOp) -> bool {
        if op.name() != self.op.as_str() { return false; }
        for (key, val) in &self.extra {
            match (key.as_str(), op) {
                ("access", LogicalOp::TransferArtifact { access, .. }) => {
                    if transfer_access_name(access) != val.as_str() { return false; }
                }
                ("native_id", LogicalOp::Native { id, .. }) => {
                    if id.as_str() != val.as_str() { return false; }
                }
                _ => return false,
            }
        }
        true
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Bindings {
    pub fields: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instruction {
    pub id: InstructionId,
    pub backend: BackendKind,
    pub provides: Vec<Capability>,
    pub requires: Vec<Capability>,
    pub matcher: OpMatcher,
    pub cost: CostHint,
    pub stability: Stability,
    pub implementation: JsonValue,
    pub bind: Bindings,
}

impl Candidate for Instruction {
    fn key(&self) -> &str { &self.matcher.op }
    fn requires(&self) -> &[Capability] { &self.requires }
    fn stability(&self) -> Stability { self.stability }
}

/// The backend's instruction catalogue is just a `select::Registry` of
/// `Instruction`s, keyed by the `LogicalOp` name each instruction matches.
pub type Catalogue = crate::select::Registry<Instruction>;

#[derive(Debug, Clone, Default)]
pub struct Policy {
    pub instruction_pins: HashMap<String, InstructionId>,
    pub allowed_instructions: Option<HashSet<InstructionId>>,
}

#[derive(Debug, Clone)]
pub struct SelectedInstruction<'a> {
    pub instruction: &'a Instruction,
    pub reason: String,
}

pub struct Selector<'a> {
    catalogue: &'a Catalogue,
    policy: Policy,
    backend: BackendKind,
}

impl<'a> Selector<'a> {
    pub fn new(catalogue: &'a Catalogue, policy: Policy, backend: BackendKind) -> Self {
        Self { catalogue, policy, backend }
    }

    pub fn for_backend(backend: &'a impl Backend) -> Self {
        Self {
            catalogue: backend.catalogue(),
            policy: Policy::default(),
            backend: backend.backend_kind(),
        }
    }

    pub fn select(
        &self,
        op: &LogicalOp,
        backend_caps: &[Capability],
        actor_caps: &[Capability],
    ) -> Option<SelectedInstruction<'_>> {
        let op_name = op.name();

        if let Some(pinned) = self.policy.instruction_pins.get(op_name)
            && let Some(instr) = self.catalogue.all().iter().find(|i| &i.id == pinned) {
                return Some(SelectedInstruction {
                    instruction: instr,
                    reason: format!("pinned by policy to {}", pinned.0),
                });
            }

        // Matching and policy exclusion are instruction-selection-specific and
        // stay here; gathering by key, the hard-requirement filter, and the
        // cost/stability ranking are the shared engine (`select`). Cost is this
        // layer's priority.
        let available: Vec<Capability> =
            backend_caps.iter().chain(actor_caps.iter()).copied().collect();
        let candidates = self.catalogue.candidates(op_name)
            .filter(|i| i.backend == self.backend)
            .filter(|i| i.matcher.matches(op))
            .filter(|i| self.policy.allowed_instructions.as_ref()
                .is_none_or(|allowed| allowed.contains(&i.id)));

        crate::select::resolve(candidates, &available, |i| i.cost.fixed)
            .map(|instr| SelectedInstruction {
                instruction: instr,
                reason: format!("selected: cost={}, stability={:?}", instr.cost.fixed, instr.stability),
            })
    }
}

// ---------------------------------------------------------------------------
// Serializable plan wire format (for external backends)
// ---------------------------------------------------------------------------

/// A serializable, backend-agnostic view of a Plan for the external backend protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendPlanRequest {
    pub workflow_name: String,
    pub units: Vec<BackendUnitRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendUnitRequest {
    pub id: String,
    pub runner: String,
    pub needs: Vec<String>,
    pub ops: Vec<JsonValue>,
}

pub fn plan_to_request(plan: &Plan) -> BackendPlanRequest {
    BackendPlanRequest {
        workflow_name: plan.workflow_name.to_string(),
        units: plan.units.iter().map(|u| BackendUnitRequest {
            id: u.id.to_string(),
            runner: u.runner.to_string(),
            needs: u.needs.iter().map(|n| n.to_string()).collect(),
            ops: u.ops.iter().map(logical_op_to_json).collect(),
        }).collect(),
    }
}

fn logical_op_to_json(op: &LogicalOp) -> JsonValue {
    match op {
        LogicalOp::CheckoutRepo => json!({"kind": "CheckoutRepo"}),
        LogicalOp::RunShell { label, script, env } => json!({
            "kind": "RunShell",
            "label": label.as_str(),
            "script": script,
            "env": env
        }),
        LogicalOp::UploadArtifact { name, path, lifetime } => json!({
            "kind": "UploadArtifact",
            "name": name.as_str(),
            "path": path,
            "lifetime": lifetime
        }),
        LogicalOp::DownloadArtifact { name, path } => json!({
            "kind": "DownloadArtifact",
            "name": name.as_str(),
            "path": path
        }),
        LogicalOp::TransferArtifact { name, path, access } => json!({
            "kind": "TransferArtifact",
            "name": name.as_str(),
            "path": path,
            "access": transfer_access_name(access)
        }),
        LogicalOp::RestoreCache { key } => json!({"kind": "RestoreCache", "key": key.as_str()}),
        LogicalOp::SaveCache { key } => json!({"kind": "SaveCache", "key": key.as_str()}),
        LogicalOp::RequestApproval { reason } => json!({"kind": "RequestApproval", "reason": reason}),
        LogicalOp::Native { id, args, fallback } => json!({
            "kind": "Native",
            "id": id.as_str(),
            "args": args,
            "fallback": fallback
        }),
    }
}

// ---------------------------------------------------------------------------
// Object-safe trait for registry and CLI use
// ---------------------------------------------------------------------------

pub struct InstructionSummary {
    pub id: String,
    pub reason: String,
}

/// Object-safe backend interface. Used by `BackendRegistry` and the CLI.
pub trait EmittingBackend: Send + Sync {
    fn id(&self) -> &str;
    fn backend_kind(&self) -> BackendKind;
    /// Instruction-selection capabilities (for `Selector`).
    fn capabilities(&self) -> Vec<Capability>;
    fn workflow_capabilities(&self) -> WorkflowCapabilities;
    fn catalogue(&self) -> &Catalogue;
    fn emit(&self, plan: &Plan) -> Result<String, Vec<Diagnostic>>;
    fn select_op(&self, op: &LogicalOp) -> Option<InstructionSummary>;
}

impl<B: Backend> EmittingBackend for B {
    fn id(&self) -> &str { self.name() }
    fn backend_kind(&self) -> BackendKind { Backend::backend_kind(self) }
    fn capabilities(&self) -> Vec<Capability> { Backend::capabilities(self) }
    fn workflow_capabilities(&self) -> WorkflowCapabilities { Backend::workflow_capabilities(self) }
    fn catalogue(&self) -> &Catalogue { Backend::catalogue(self) }
    fn emit(&self, plan: &Plan) -> Result<String, Vec<Diagnostic>> { Backend::emit(self, plan) }
    fn select_op(&self, op: &LogicalOp) -> Option<InstructionSummary> {
        let sel = Selector::for_backend(self);
        let caps = Backend::capabilities(self);
        sel.select(op, &caps, &[]).map(|s| InstructionSummary {
            id: s.instruction.id.0.to_string(),
            reason: s.reason,
        })
    }
}

// ---------------------------------------------------------------------------
// Generic Backend trait (type-safe, not object-safe)
// ---------------------------------------------------------------------------

pub use renderers::Renderer;

pub trait Backend: Send + Sync {
    type Ir;
    type Options: Default;

    fn name(&self) -> &str;
    fn backend_kind(&self) -> BackendKind;
    /// Instruction-selection capabilities (e.g. "github.action_calls.uses").
    fn capabilities(&self) -> Vec<Capability>;
    fn catalogue(&self) -> &Catalogue;
    fn options(&self) -> &Self::Options;

    fn workflow_capabilities(&self) -> WorkflowCapabilities {
        WorkflowCapabilities::JOBS
            | WorkflowCapabilities::DEPENDENCIES
            | WorkflowCapabilities::SECRETS
    }

    fn lower(&self, plan: &Plan) -> Self::Ir;
    fn render(&self, ir: &Self::Ir) -> String;

    fn emit(&self, plan: &Plan) -> Result<String, Vec<Diagnostic>> {
        Ok(self.render(&self.lower(plan)))
    }

    fn selector(&self) -> Selector<'_> where Self: Sized {
        Selector::for_backend(self)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn transfer_access_name(access: &AccessMode) -> &'static str {
    match access {
        AccessMode::Copy => "Copy",
        AccessMode::MountReadOnly => "MountReadOnly",
        AccessMode::MountReadWrite => "MountReadWrite",
        AccessMode::Stream => "Stream",
        AccessMode::SameHostPath => "SameHostPath",
        AccessMode::OciLayer => "OciLayer",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_instr(id: &str, op: &str, backend: BackendKind, requires: Vec<Capability>, cost: u32) -> Instruction {
        Instruction {
            id: InstructionId(id.into()),
            backend,
            provides: vec![],
            requires,
            matcher: OpMatcher::for_op(op),
            cost: CostHint { fixed: cost, per_mb: 0 },
            stability: Stability::Stable,
            implementation: json!({}),
            bind: Bindings::default(),
        }
    }

    #[test]
    fn selects_matching_instruction() {
        let cat = Catalogue::from_items(vec![
            make_instr("github.checkout", "CheckoutRepo", BackendKind::Github,
                vec![Capability::new("github.action_calls.uses")], 5),
        ]);
        let sel = Selector::new(&cat, Policy::default(), BackendKind::Github);
        let caps = vec![Capability::new("github.action_calls.uses")];
        let result = sel.select(&LogicalOp::CheckoutRepo, &caps, &[]);
        assert!(result.is_some());
        assert_eq!(result.unwrap().instruction.id.0, "github.checkout");
    }

    #[test]
    fn filters_by_missing_capability() {
        let cat = Catalogue::from_items(vec![
            make_instr("github.checkout", "CheckoutRepo", BackendKind::Github,
                vec![Capability::new("github.action_calls.uses")], 5),
        ]);
        let sel = Selector::new(&cat, Policy::default(), BackendKind::Github);
        assert!(sel.select(&LogicalOp::CheckoutRepo, &[], &[]).is_none());
    }

    #[test]
    fn filters_by_backend_kind() {
        let cat = Catalogue::from_items(vec![
            make_instr("local.checkout", "CheckoutRepo", BackendKind::Local, vec![], 3),
        ]);
        let sel = Selector::new(&cat, Policy::default(), BackendKind::Github);
        assert!(sel.select(&LogicalOp::CheckoutRepo, &[], &[]).is_none());
    }

    #[test]
    fn policy_pin_overrides_ranking() {
        let cat = Catalogue::from_items(vec![
            make_instr("a", "CheckoutRepo", BackendKind::Github, vec![], 5),
            make_instr("b", "CheckoutRepo", BackendKind::Github, vec![], 1),
        ]);
        let mut policy = Policy::default();
        policy.instruction_pins.insert("CheckoutRepo".into(), InstructionId("a".into()));
        let sel = Selector::new(&cat, policy, BackendKind::Github);
        assert_eq!(sel.select(&LogicalOp::CheckoutRepo, &[], &[]).unwrap().instruction.id.0, "a");
    }

    #[test]
    fn ranks_by_cost_ascending() {
        let cat = Catalogue::from_items(vec![
            make_instr("expensive", "RunShell", BackendKind::Github, vec![], 10),
            make_instr("cheap", "RunShell", BackendKind::Github, vec![], 1),
        ]);
        let sel = Selector::new(&cat, Policy::default(), BackendKind::Github);
        let op = LogicalOp::RunShell { label: "x".into(), script: "echo hi".into(), env: Default::default() };
        assert_eq!(sel.select(&op, &[], &[]).unwrap().instruction.id.0, "cheap");
    }

    #[test]
    fn allowlist_rejects_unlisted_instruction() {
        let cat = Catalogue::from_items(vec![
            make_instr("github.checkout", "CheckoutRepo", BackendKind::Github, vec![], 5),
        ]);
        let policy = Policy { allowed_instructions: Some(HashSet::new()), ..Default::default() };
        let sel = Selector::new(&cat, policy, BackendKind::Github);
        assert!(sel.select(&LogicalOp::CheckoutRepo, &[], &[]).is_none());
    }

    #[test]
    fn catalogue_indexes_instructions_by_op() {
        let cat = Catalogue::from_items(vec![
            make_instr("a", "RunShell", BackendKind::Github, vec![], 1),
            make_instr("b", "RunShell", BackendKind::Local, vec![], 1),
            make_instr("c", "CheckoutRepo", BackendKind::Github, vec![], 1),
        ]);
        assert_eq!(cat.candidates("RunShell").count(), 2);
        assert_eq!(cat.candidates("CheckoutRepo").count(), 1);
        assert_eq!(cat.candidates("Nope").count(), 0);
        assert_eq!(cat.all().len(), 3);
    }

    #[test]
    fn op_matcher_extra_predicate_access() {
        let mut matcher = OpMatcher::for_op("TransferArtifact");
        matcher.extra.insert("access".into(), "Copy".into());
        let copy_op = LogicalOp::TransferArtifact { name: "x".into(), path: None, access: AccessMode::Copy };
        let mount_op = LogicalOp::TransferArtifact { name: "x".into(), path: None, access: AccessMode::MountReadOnly };
        assert!(matcher.matches(&copy_op));
        assert!(!matcher.matches(&mount_op));
    }

    #[test]
    fn derive_capability_profile_from_actors_and_placements() {
        use crate::ir::{Actor, Inventory, InventoryPlacement};
        let inv = Inventory {
            id: "test".into(),
            actors: vec![Actor {
                id: "local".into(),
                labels: vec!["ubuntu".into()],
                capabilities: vec!["linux".into(), "cache_mount_access".into()],
                resources: None,
            }],
            placements: vec![InventoryPlacement {
                id: "workspace".into(),
                kind: "volume".into(),
                access_modes: vec!["same_host".into(), "mount_rw".into()],
                accessible_by: vec![],
            }],
            implementations: vec![],
        };
        let profile = derive_capability_profile_from_inventory(Some(&inv));
        assert!(profile.contains(BackendCapabilities::MOUNTS));
        assert!(profile.contains(BackendCapabilities::COLOCATION));
        assert!(!profile.contains(BackendCapabilities::CACHE));
    }

    #[test]
    fn derive_capability_profile_cache_from_persistent_placement() {
        use crate::ir::{Inventory, InventoryPlacement};
        let inv = Inventory {
            id: "test".into(),
            actors: vec![],
            placements: vec![InventoryPlacement {
                id: "cache".into(),
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

    #[test]
    fn derive_capability_profile_empty_without_inventory() {
        let profile = derive_capability_profile_from_inventory(None);
        assert_eq!(profile, BackendCapabilities::empty());
    }

    #[test]
    fn plan_to_request_serializes_all_ops() {
        use crate::ir::{ArtifactType, WorkflowBuilder};
        use crate::planner::plan;
        let mut b = WorkflowBuilder::new("test");
        let src = b.artifact("src", ArtifactType::SourceTree);
        let bin = b.artifact("bin", ArtifactType::Binary);
        b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        b.shell_action("build", "build", &[src], &[bin], "cargo build");
        b.actor("r", &["ubuntu-latest"], &[]);
        let p = plan(&b.build()).unwrap();
        let req = plan_to_request(&p);
        assert_eq!(req.workflow_name, "test");
        assert_eq!(req.units.len(), 2);
        let build_unit = req.units.iter().find(|u| u.id == "build").unwrap();
        assert!(build_unit.ops.iter().any(|op| op["kind"] == "DownloadArtifact"));
        assert!(build_unit.ops.iter().any(|op| op["kind"] == "RunShell"));
        assert!(build_unit.ops.iter().any(|op| op["kind"] == "UploadArtifact"));
    }
}

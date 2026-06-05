pub mod renderers;
pub mod github;
pub mod local;

use crate::diagnostics::Diagnostic;
use crate::planner::{PhysicalOp, Plan, AccessMode};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use ustr::Ustr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackendKind {
    Github,
    Local,
    Gitlab,
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capability(pub Ustr);

impl Capability {
    pub fn new(s: impl AsRef<str>) -> Self { Self(Ustr::from(s.as_ref())) }
}

bitflags::bitflags! {
    /// High-level capabilities the optimizer reasons about (distinct from the
    /// instruction-selection `Capability` strings). Decides which placement/runner
    /// optimizations are *legal* for a backend; default is empty (none), which
    /// forces the copy/upload-download fallback.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct BackendCapabilities: u32 {
        const MOUNTS     = 1 << 0;
        const COLOCATION = 1 << 1;
        const CACHE      = 1 << 2;
        const STREAM     = 1 << 3;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stability {
    Experimental,
    Beta,
    Stable,
    Deprecated,
}

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

    pub fn matches(&self, op: &PhysicalOp) -> bool {
        if op.name() != self.op.as_str() { return false; }
        for (key, val) in &self.extra {
            match (key.as_str(), op) {
                ("access", PhysicalOp::TransferArtifact { access, .. }) => {
                    if transfer_access_name(access) != val.as_str() { return false; }
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

pub struct Catalogue {
    entries: Vec<Instruction>,
    index: InstructionIndex,
}

impl Catalogue {
    pub fn new(entries: Vec<Instruction>) -> Self {
        let index = InstructionIndex::build(&entries);
        Self { entries, index }
    }

    pub fn entries(&self) -> &[Instruction] { &self.entries }
    pub fn index(&self) -> &InstructionIndex { &self.index }
}

/// Precomputed lookup tables over a catalogue's entries. Each map stores
/// indices into the entries slice, grouped by op name, backend, and provided
/// capability, so the selector can skip a full scan.
pub struct InstructionIndex {
    by_op: HashMap<String, Vec<usize>>,
    by_backend: HashMap<BackendKind, Vec<usize>>,
    by_capability: HashMap<Capability, Vec<usize>>,
}

impl InstructionIndex {
    fn build(entries: &[Instruction]) -> Self {
        let mut by_op: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_backend: HashMap<BackendKind, Vec<usize>> = HashMap::new();
        let mut by_capability: HashMap<Capability, Vec<usize>> = HashMap::new();
        for (i, instr) in entries.iter().enumerate() {
            by_op.entry(instr.matcher.op.clone()).or_default().push(i);
            by_backend.entry(instr.backend.clone()).or_default().push(i);
            for cap in &instr.provides {
                by_capability.entry(*cap).or_default().push(i);
            }
        }
        Self { by_op, by_backend, by_capability }
    }

    pub fn by_op(&self, op_name: &str) -> &[usize] {
        self.by_op.get(op_name).map_or(&[], Vec::as_slice)
    }

    pub fn by_backend(&self, backend: &BackendKind) -> &[usize] {
        self.by_backend.get(backend).map_or(&[], Vec::as_slice)
    }

    pub fn by_capability(&self, cap: &Capability) -> &[usize] {
        self.by_capability.get(cap).map_or(&[], Vec::as_slice)
    }
}

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
        op: &PhysicalOp,
        backend_caps: &[Capability],
        actor_caps: &[Capability],
    ) -> Option<SelectedInstruction<'_>> {
        let op_name = op.name();

        if let Some(pinned) = self.policy.instruction_pins.get(op_name)
            && let Some(instr) = self.catalogue.entries.iter().find(|i| &i.id == pinned) {
                return Some(SelectedInstruction {
                    instruction: instr,
                    reason: format!("pinned by policy to {}", pinned.0),
                });
            }

        let mut candidates: Vec<&Instruction> = self.catalogue.index.by_op(op_name).iter()
            .map(|&i| &self.catalogue.entries[i])
            .filter(|i| i.backend == self.backend)
            .filter(|i| i.matcher.matches(op))
            .filter(|i| i.requires.iter().all(|req| {
                backend_caps.iter().chain(actor_caps.iter()).any(|cap| cap == req)
            }))
            .filter(|i| self.policy.allowed_instructions.as_ref()
                .is_none_or(|allowed| allowed.contains(&i.id)))
            .collect();

        candidates.sort_by(|a, b| {
            a.cost.fixed.cmp(&b.cost.fixed)
                .then_with(|| stability_ord(a.stability).cmp(&stability_ord(b.stability)))
        });

        candidates.first().map(|instr| SelectedInstruction {
            instruction: instr,
            reason: format!("selected: cost={}, stability={:?}", instr.cost.fixed, instr.stability),
        })
    }
}

pub use renderers::Renderer;

/// A backend turns a `Plan` into backend-specific output. The contract is
/// codified: `lower` selects instructions and builds the backend IR, `render`
/// turns that IR into text via a `Renderer`, and `emit` is their composition.
pub trait Backend: Send + Sync {
    /// The backend's structured intermediate representation (e.g. a workflow
    /// YAML model or a bash-script model).
    type Ir;
    /// Backend-specific options (e.g. push branches, default image). Populated
    /// by the frontend/CLI; the engine only needs them to be `Default`.
    type Options: Default;

    fn name(&self) -> &str;
    fn backend_kind(&self) -> BackendKind;
    fn capabilities(&self) -> Vec<Capability>;
    fn catalogue(&self) -> &Catalogue;
    fn options(&self) -> &Self::Options;

    /// High-level capabilities the optimizer uses for legality (mounts, etc.).
    /// Conservative by default — backends opt in to richer placement.
    fn capability_profile(&self) -> BackendCapabilities { BackendCapabilities::default() }

    /// Lower a plan into the backend IR: instruction selection + structure.
    fn lower(&self, plan: &Plan) -> Self::Ir;
    /// Render the backend IR to text. Implemented via a `Renderer`.
    fn render(&self, ir: &Self::Ir) -> String;

    /// Produce final backend output. Always `render ∘ lower`.
    fn emit(&self, plan: &Plan) -> Result<String, Vec<Diagnostic>> {
        Ok(self.render(&self.lower(plan)))
    }

    /// A `Selector` over this backend's catalogue (instruction selection helper).
    fn selector(&self) -> Selector<'_> where Self: Sized {
        Selector::for_backend(self)
    }
}

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

fn stability_ord(s: Stability) -> u8 {
    match s {
        Stability::Stable => 0,
        Stability::Beta => 1,
        Stability::Experimental => 2,
        Stability::Deprecated => 3,
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
        let cat = Catalogue::new(vec![
            make_instr("github.checkout", "CheckoutRepo", BackendKind::Github,
                vec![Capability::new("github.actions.uses")], 5),
        ]);
        let sel = Selector::new(&cat, Policy::default(), BackendKind::Github);
        let caps = vec![Capability::new("github.actions.uses")];
        let result = sel.select(&PhysicalOp::CheckoutRepo, &caps, &[]);
        assert!(result.is_some());
        assert_eq!(result.unwrap().instruction.id.0, "github.checkout");
    }

    #[test]
    fn filters_by_missing_capability() {
        let cat = Catalogue::new(vec![
            make_instr("github.checkout", "CheckoutRepo", BackendKind::Github,
                vec![Capability::new("github.actions.uses")], 5),
        ]);
        let sel = Selector::new(&cat, Policy::default(), BackendKind::Github);
        assert!(sel.select(&PhysicalOp::CheckoutRepo, &[], &[]).is_none());
    }

    #[test]
    fn filters_by_backend_kind() {
        let cat = Catalogue::new(vec![
            make_instr("local.checkout", "CheckoutRepo", BackendKind::Local, vec![], 3),
        ]);
        let sel = Selector::new(&cat, Policy::default(), BackendKind::Github);
        assert!(sel.select(&PhysicalOp::CheckoutRepo, &[], &[]).is_none());
    }

    #[test]
    fn policy_pin_overrides_ranking() {
        let cat = Catalogue::new(vec![
            make_instr("a", "CheckoutRepo", BackendKind::Github, vec![], 5),
            make_instr("b", "CheckoutRepo", BackendKind::Github, vec![], 1),
        ]);
        let mut policy = Policy::default();
        policy.instruction_pins.insert("CheckoutRepo".into(), InstructionId("a".into()));
        let sel = Selector::new(&cat, policy, BackendKind::Github);
        assert_eq!(sel.select(&PhysicalOp::CheckoutRepo, &[], &[]).unwrap().instruction.id.0, "a");
    }

    #[test]
    fn ranks_by_cost_ascending() {
        let cat = Catalogue::new(vec![
            make_instr("expensive", "RunShell", BackendKind::Github, vec![], 10),
            make_instr("cheap", "RunShell", BackendKind::Github, vec![], 1),
        ]);
        let sel = Selector::new(&cat, Policy::default(), BackendKind::Github);
        let op = PhysicalOp::RunShell { label: "x".into(), script: "echo hi".into(), env: Default::default() };
        assert_eq!(sel.select(&op, &[], &[]).unwrap().instruction.id.0, "cheap");
    }

    #[test]
    fn allowlist_rejects_unlisted_instruction() {
        let cat = Catalogue::new(vec![
            make_instr("github.checkout", "CheckoutRepo", BackendKind::Github, vec![], 5),
        ]);
        let mut policy = Policy::default();
        policy.allowed_instructions = Some(HashSet::new());
        let sel = Selector::new(&cat, policy, BackendKind::Github);
        assert!(sel.select(&PhysicalOp::CheckoutRepo, &[], &[]).is_none());
    }

    #[test]
    fn index_groups_by_op_backend_and_capability() {
        let cat = Catalogue::new(vec![
            make_instr("a", "RunShell", BackendKind::Github, vec![], 1),
            make_instr("b", "RunShell", BackendKind::Local, vec![], 1),
            make_instr("c", "CheckoutRepo", BackendKind::Github, vec![], 1),
        ]);
        let idx = cat.index();
        assert_eq!(idx.by_op("RunShell").len(), 2);
        assert_eq!(idx.by_op("CheckoutRepo").len(), 1);
        assert_eq!(idx.by_backend(&BackendKind::Github).len(), 2);
        assert_eq!(idx.by_backend(&BackendKind::Local).len(), 1);
    }

    #[test]
    fn index_by_capability_finds_providers() {
        let mut p = make_instr("p", "RunShell", BackendKind::Github, vec![], 1);
        p.provides = vec![Capability::new("artifact.upload")];
        let cat = Catalogue::new(vec![p]);
        let providers = cat.index().by_capability(&Capability::new("artifact.upload"));
        assert_eq!(providers, &[0]);
        assert!(cat.index().by_capability(&Capability::new("nonexistent")).is_empty());
    }

    #[test]
    fn index_returns_empty_slice_for_unknown_keys() {
        let cat = Catalogue::new(vec![]);
        assert!(cat.index().by_op("RunShell").is_empty());
        assert!(cat.index().by_backend(&BackendKind::Github).is_empty());
    }

    #[test]
    fn op_matcher_extra_predicate_access() {
        let mut matcher = OpMatcher::for_op("TransferArtifact");
        matcher.extra.insert("access".into(), "Copy".into());
        let copy_op = PhysicalOp::TransferArtifact { name: "x".into(), path: None, access: AccessMode::Copy };
        let mount_op = PhysicalOp::TransferArtifact { name: "x".into(), path: None, access: AccessMode::MountReadOnly };
        assert!(matcher.matches(&copy_op));
        assert!(!matcher.matches(&mount_op));
    }
}

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ustr::Ustr;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub u32);
        impl $name {
            #[inline]
            pub fn idx(self) -> usize { self.0 as usize }
        }
        impl From<usize> for $name {
            fn from(n: usize) -> Self { Self(n as u32) }
        }
    };
}

id_type!(ArtifactId);
id_type!(ActionCallId);
id_type!(ConsequenceId);
id_type!(ActorId);
id_type!(PlacementId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactType {
    SourceTree,
    Wheel,
    Binary,
    ContainerImage,
    Sbom,
    Signature,
    ReleaseBundle,
    TestReport,
    CoverageData,
    DocsSite,
    ProfileData,
    Model,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub name: Ustr,
    pub ty: ArtifactType,
    /// Index of the action that produces this artifact.
    /// `None` means externally supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<ActionCallId>,
    /// Workspace-relative path the artifact occupies (e.g. "target/release/app").
    /// Backends use it for upload/download; under `loom test` it drives the mock
    /// artifact store. `None` artifacts are transferred but not materialized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Retention requirement: how long the artifact should exist. A category
    /// ("ephemeral", "workflow", "release", "permanent") or a duration ("14d").
    /// Distinct from placement persistence (a storage capability); this is the
    /// semantic retention requirement. `None` means the backend default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsequenceKind {
    Network,
    SecretAccess,
    GitWrite,
    PublishRelease,
    Deployment,
    CommentOnPr,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consequence {
    pub name: Ustr,
    pub kind: ConsequenceKind,
    pub requires_approval: bool,
}

/// Concurrency control. Members of the same `group` are coordinated: with
/// `cancel_in_progress`, a new run cancels an in-progress one (cancel-previous);
/// without it, runs are mutually exclusive and queue (exclusive). Applies to a
/// whole workflow or to a single action. Lowers to backend-native mechanisms
/// (GitHub concurrency groups, GitLab resource groups, local locks).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coordination {
    pub group: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_in_progress: bool,
}

/// Execution resources an action requires or an actor advertises. Participates
/// in actor selection: an action is assignable to an actor only if the actor
/// satisfies every requirement. Not emitted directly; it constrains which actor
/// (and thus which runner) is chosen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resources {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<u32>,
}

impl Resources {
    pub fn is_empty(&self) -> bool {
        self.cpu.is_none() && self.memory.is_none() && self.disk.is_none() && self.gpu.is_none()
    }
}

impl Actor {
    /// True if this actor's advertised resources meet every requirement in
    /// `req`. An actor that advertises nothing satisfies only an empty
    /// requirement. Memory/disk strings are compared as byte sizes.
    pub fn satisfies(&self, req: &Resources) -> bool {
        let have = self.resources.clone().unwrap_or_default();
        if let Some(c) = req.cpu
            && have.cpu.unwrap_or(0) < c { return false; }
        if let Some(g) = req.gpu
            && have.gpu.unwrap_or(0) < g { return false; }
        if let Some(m) = &req.memory {
            let need = parse_size_bytes(m).unwrap_or(0);
            if have.memory.as_deref().and_then(parse_size_bytes).unwrap_or(0) < need { return false; }
        }
        if let Some(d) = &req.disk {
            let need = parse_size_bytes(d).unwrap_or(0);
            if have.disk.as_deref().and_then(parse_size_bytes).unwrap_or(0) < need { return false; }
        }
        true
    }
}

/// Parse a size like "32Gi", "512Mi", "2G", "1024" into bytes. Binary units
/// (Ki/Mi/Gi/Ti) use 1024; decimal (K/M/G/T) use 1000; a bare number is bytes.
pub fn parse_size_bytes(s: &str) -> Option<u64> {
    let s = s.trim();
    match s.find(|c: char| !c.is_ascii_digit()) {
        None => s.parse().ok(),
        Some(i) => {
            let n: u64 = s[..i].parse().ok()?;
            let mult: u64 = match &s[i..] {
                "Ki" => 1 << 10,
                "Mi" => 1 << 20,
                "Gi" => 1 << 30,
                "Ti" => 1 << 40,
                "K" | "k" => 1_000,
                "M" => 1_000_000,
                "G" => 1_000_000_000,
                "T" => 1_000_000_000_000,
                "B" | "" => 1,
                _ => return None,
            };
            Some(n.saturating_mul(mult))
        }
    }
}

/// An execution resource declared in an Inventory.
/// `id` is the stable identifier; `labels` are the backend-specific runner
/// selector strings (e.g. `ubuntu-latest`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    pub id: Ustr,
    pub labels: Vec<Ustr>,
    /// Open-ended capability names this actor provides (e.g. "mount", "gpu").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<Ustr>,
    /// Execution resources this actor advertises; checked against action
    /// requirements during actor selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<Resources>,
}

/// A storage or transfer resource declared in an Inventory.
/// `access_modes` are open-ended strings: "copy", "mount_ro", "mount_rw",
/// "stream", "persistent", "same_host", "cross_host".
/// `accessible_by` lists actor IDs that can use this placement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryPlacement {
    pub id: Ustr,
    pub kind: Ustr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub access_modes: Vec<Ustr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accessible_by: Vec<Ustr>,
}

/// A technology available to realize semantic actions (e.g. "git", "maturin").
/// `prefer` biases lowering selection toward this implementation.
/// `deny` excludes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryImpl {
    pub id: Ustr,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Ustr>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prefer: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deny: bool,
}

/// Describes available execution and storage resources for the planner.
/// Embedded directly in Thread IR so planning is self-contained.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub id: Ustr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<Actor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub placements: Vec<InventoryPlacement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implementations: Vec<InventoryImpl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlacementStrategy {
    GithubArtifact,
    SharedVolume { path: String },
    PersistentCache { key: String },
    LocalPath { path: String },
    OciRegistry { registry: String, tag: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Placement {
    pub artifact: ArtifactId,
    pub strategy: PlacementStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActorConstraint {
    Specific(ActorId),
    Label(Ustr),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum CaptureRule {
    #[default]
    NoCapture,
    Stdout,
    Stderr,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellAction {
    pub script: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    pub capture: CaptureRule,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionCall {
    pub name: Ustr,
    pub action: Ustr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ArtifactId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<ArtifactId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consequences: Vec<ConsequenceId>,
    /// Secrets this action needs. How they are consumed is up to the selected
    /// instruction: shell injects env vars, a `uses` step passes `with:` inputs, etc.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<Ustr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actor_constraints: Vec<ActorConstraint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<ShellAction>,
    /// Maximum execution duration (e.g. "30m"). Backend-independent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
    /// Action-level concurrency control (e.g. one production deploy at a time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordination: Option<Coordination>,
    /// Execution resources this action requires; constrains actor selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<Resources>,
}

/// How an op port is used: typed artifact or untyped scalar parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortKind {
    Artifact,
    Scalar,
}

/// A single named input or output of an ActionDef.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Port {
    pub name: Ustr,
    /// Artifact type string (e.g. "Wheel") or scalar type ("string", "int").
    pub ty: String,
    pub kind: PortKind,
}

/// A concrete implementation strategy for an op. Variants are execution
/// primitives that any backend can reason about. Backend-specific native steps
/// are not modeled here: they are an emit-time concern handled by a `Native`
/// LogicalOp plus the backend catalogue.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Implementation {
    Shell {
        run: String,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        env: HashMap<String, String>,
        #[serde(default)]
        capture: CaptureRule,
    },
    Container {
        image: String,
        run: String,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        env: HashMap<String, String>,
    },
    /// A VCS checkout (the planner maps this to LogicalOp::CheckoutRepo).
    Checkout,
    /// A high-level semantic action (e.g. "build.python_wheel"). The planner's
    /// backend-agnostic lowering selects a concrete implementation from the
    /// inventory and renders it. Args carry the action's configuration.
    Semantic {
        op: String,
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        args: std::collections::BTreeMap<String, serde_json::Value>,
        /// Pin the implementation for this call (e.g. "pytest"), overriding
        /// inventory-based ranking. Use when one inventory offers several
        /// runners for the same action. Must still be available and not denied.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        using: Option<String>,
        /// Soft, ordered scoped preferences (from a `with impl(...)`/`impls([...])`
        /// block). The first listed implementation that is an eligible candidate
        /// for the action is chosen; if none applies, normal ranking proceeds.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        prefer: Vec<String>,
    },
}

/// A reusable typed operation definition. Actions reference an op by id.
/// When a workflow includes ActionDefs, the planner and validator use them
/// to type-check calls and select implementations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDef {
    pub id: Ustr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<Port>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<Port>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consequences: Vec<ConsequenceKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implementations: Vec<Implementation>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

/// A single optimization objective to minimize. The planner/optimizer breaks
/// ties between candidate plans by the workflow's objective order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Objective {
    /// Wall-clock latency along the plan's critical path.
    CriticalPath,
    /// Total bytes moved between units.
    TransferBytes,
    /// Estimated monetary cost.
    DollarCost,
}

/// Default objective priority: latency, then bytes, then dollars.
pub fn default_objectives() -> Vec<Objective> {
    vec![Objective::CriticalPath, Objective::TransferBytes, Objective::DollarCost]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policies {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallel_jobs: Option<usize>,
    /// Tie-break priority for the optimizer's cost comparison.
    #[serde(default = "default_objectives")]
    pub objectives: Vec<Objective>,
    /// When true, backends install each job's declared tool dependencies on job
    /// start. Default false: the execution environment is assumed to provide them.
    #[serde(default)]
    pub install_dependencies: bool,
}

impl Default for Policies {
    fn default() -> Self {
        Self { max_parallel_jobs: None, objectives: default_objectives(), install_dependencies: false }
    }
}

/// How a workflow begins execution. A trigger controls workflow *entry*; it is
/// distinct from a condition, which controls execution after entry, and from
/// the `EventContext` the planner uses to gate consequences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    PullRequest,
    Push {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        branches: Vec<String>,
    },
    Tag {
        pattern: String,
    },
    Schedule {
        cron: String,
    },
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub name: Ustr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_calls: Vec<ActionCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consequences: Vec<Consequence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub placements: Vec<Placement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory: Option<Inventory>,
    #[serde(default)]
    pub policies: Policies,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_defs: Vec<ActionDef>,
    /// How this workflow is entered. Empty means the backend's default trigger.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<Trigger>,
    /// Workflow-level concurrency control (e.g. one release at a time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordination: Option<Coordination>,
}

impl Workflow {
    pub fn find_action_def(&self, id: &str) -> Option<&ActionDef> {
        self.action_defs.iter().find(|o| o.id.as_str() == id)
    }

    /// Returns the actors from the inventory, or an empty slice if absent.
    pub fn actors(&self) -> &[Actor] {
        self.inventory.as_ref().map(|inv| inv.actors.as_slice()).unwrap_or(&[])
    }
}

impl Workflow {
    #[inline]
    pub fn artifact(&self, id: ArtifactId) -> &Artifact { &self.artifacts[id.idx()] }
    #[inline]
    pub fn action_call(&self, id: ActionCallId) -> &ActionCall { &self.action_calls[id.idx()] }
    #[inline]
    pub fn consequence(&self, id: ConsequenceId) -> &Consequence { &self.consequences[id.idx()] }
    #[inline]
    pub fn actor(&self, id: ActorId) -> &Actor {
        self.inventory.as_ref()
            .and_then(|inv| inv.actors.get(id.idx()))
            .expect("ActorId references missing inventory actor")
    }
}

#[derive(Default)]
pub struct WorkflowBuilder {
    name: Ustr,
    artifacts: Vec<Artifact>,
    action_calls: Vec<ActionCall>,
    consequences: Vec<Consequence>,
    placements: Vec<Placement>,
    inv_actors: Vec<Actor>,
    inv_placements: Vec<InventoryPlacement>,
    inv_implementations: Vec<InventoryImpl>,
    policies: Policies,
    action_defs: Vec<ActionDef>,
    triggers: Vec<Trigger>,
    coordination: Option<Coordination>,
}

impl WorkflowBuilder {
    pub fn new(name: &str) -> Self {
        Self { name: name.into(), ..Default::default() }
    }

    /// Declare an artifact; returns its ID.
    pub fn artifact(&mut self, name: &str, ty: ArtifactType) -> ArtifactId {
        let id = ArtifactId(self.artifacts.len() as u32);
        self.artifacts.push(Artifact { name: name.into(), ty, producer: None, path: None, lifetime: None });
        id
    }

    /// Declare an artifact with a workspace path; returns its ID.
    pub fn artifact_at(&mut self, name: &str, ty: ArtifactType, path: &str) -> ArtifactId {
        let id = ArtifactId(self.artifacts.len() as u32);
        self.artifacts.push(Artifact { name: name.into(), ty, producer: None, path: Some(path.into()), lifetime: None });
        id
    }

    /// Declare an action. Sets `producer` on each output artifact automatically.
    pub fn action(
        &mut self,
        name: &str,
        action_def: &str,
        inputs: &[ArtifactId],
        outputs: &[ArtifactId],
    ) -> ActionCallId {
        let id = ActionCallId(self.action_calls.len() as u32);
        self.action_calls.push(ActionCall {
            name: name.into(),
            action: action_def.into(),
            inputs: inputs.to_vec(),
            outputs: outputs.to_vec(),
            consequences: vec![],
            secrets: vec![],
            actor_constraints: vec![],
            shell: None,
            timeout: None,
            coordination: None,
            resources: None,
        });
        for &art in outputs {
            self.artifacts[art.idx()].producer = Some(id);
        }
        id
    }

    /// Like `action` but also attaches a shell script.
    pub fn shell_action(
        &mut self,
        name: &str,
        action_def: &str,
        inputs: &[ArtifactId],
        outputs: &[ArtifactId],
        script: &str,
    ) -> ActionCallId {
        let id = self.action(name, action_def, inputs, outputs);
        self.action_calls[id.idx()].shell = Some(ShellAction {
            script: script.into(),
            env: HashMap::new(),
            capture: CaptureRule::NoCapture,
        });
        id
    }

    pub fn add_consequence_to(&mut self, action: ActionCallId, consequence: ConsequenceId) {
        self.action_calls[action.idx()].consequences.push(consequence);
    }

    /// Declare secrets an action needs, by name.
    pub fn add_secrets(&mut self, action: ActionCallId, names: &[&str]) {
        self.action_calls[action.idx()].secrets
            .extend(names.iter().map(|s| Ustr::from(s)));
    }

    /// Pin an action to a specific actor.
    pub fn constrain_actor(&mut self, action: ActionCallId, actor: ActorId) {
        self.action_calls[action.idx()].actor_constraints.push(ActorConstraint::Specific(actor));
    }

    /// Constrain an action to actors carrying a label (a soft constraint: the
    /// optimizer may pick any actor with this label).
    pub fn constrain_label(&mut self, action: ActionCallId, label: &str) {
        self.action_calls[action.idx()].actor_constraints.push(ActorConstraint::Label(Ustr::from(label)));
    }

    /// Declare a placement strategy for an artifact.
    pub fn place(&mut self, artifact: ArtifactId, strategy: PlacementStrategy) {
        self.placements.push(Placement { artifact, strategy });
    }

    pub fn consequence(&mut self, name: &str, kind: ConsequenceKind, requires_approval: bool) -> ConsequenceId {
        let id = ConsequenceId(self.consequences.len() as u32);
        self.consequences.push(Consequence { name: name.into(), kind, requires_approval });
        id
    }

    /// Declare an actor in the workflow's default inventory.
    pub fn actor(&mut self, id: &str, labels: &[&str], capabilities: &[&str]) -> ActorId {
        let actor_id = ActorId(self.inv_actors.len() as u32);
        self.inv_actors.push(Actor {
            id: id.into(),
            labels: labels.iter().map(|s| Ustr::from(s)).collect(),
            capabilities: capabilities.iter().map(|s| Ustr::from(s)).collect(),
            resources: None,
        });
        actor_id
    }

    /// Declare an available implementation technology in the workflow's default inventory.
    pub fn implementation(&mut self, id: &str, version: Option<&str>, prefer: bool, deny: bool) {
        self.inv_implementations.push(InventoryImpl {
            id: id.into(),
            version: version.map(|s| s.into()),
            prefer,
            deny,
        });
    }

    /// Declare a placement provider in the workflow's default inventory.
    pub fn inventory_placement(
        &mut self,
        id: &str,
        kind: &str,
        access_modes: &[&str],
        accessible_by: &[&str],
    ) {
        self.inv_placements.push(InventoryPlacement {
            id: id.into(),
            kind: kind.into(),
            access_modes: access_modes.iter().map(|s| Ustr::from(s)).collect(),
            accessible_by: accessible_by.iter().map(|s| Ustr::from(s)).collect(),
        });
    }

    pub fn max_parallel_jobs(&mut self, n: usize) -> &mut Self {
        self.policies.max_parallel_jobs = Some(n);
        self
    }

    pub fn define_action(&mut self, def: ActionDef) {
        self.action_defs.push(def);
    }

    pub fn trigger(&mut self, t: Trigger) -> &mut Self {
        self.triggers.push(t);
        self
    }

    pub fn build(self) -> Workflow {
        let inventory = if !self.inv_actors.is_empty() || !self.inv_placements.is_empty() || !self.inv_implementations.is_empty() {
            Some(Inventory {
                id: "default".into(),
                actors: self.inv_actors,
                placements: self.inv_placements,
                implementations: self.inv_implementations,
            })
        } else {
            None
        };
        Workflow {
            name: self.name,
            artifacts: self.artifacts,
            action_calls: self.action_calls,
            consequences: self.consequences,
            placements: self.placements,
            inventory,
            policies: self.policies,
            action_defs: self.action_defs,
            triggers: self.triggers,
            coordination: self.coordination,
        }
    }
}

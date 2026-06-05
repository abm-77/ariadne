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
id_type!(ActionId);
id_type!(EffectId);
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
    pub producer: Option<ActionId>,
    /// Workspace-relative path the artifact occupies (e.g. "target/release/app").
    /// Backends use it for upload/download; under `loom test` it drives the mock
    /// artifact store. `None` artifacts are transferred but not materialized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectKind {
    Network,
    SecretAccess,
    GitWrite,
    PublishRelease,
    Deployment,
    CommentOnPr,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Effect {
    pub name: Ustr,
    pub kind: EffectKind,
    pub requires_approval: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    pub name: Ustr,
    pub labels: Vec<Ustr>,
    /// Open-ended capability names this actor provides (e.g. "mount", "gpu").
    /// Capabilities are defined by backends/operators, not hardcoded in the IR.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<Ustr>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CaptureRule {
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
pub struct Action {
    pub name: Ustr,
    pub op: Ustr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ArtifactId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<ArtifactId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectId>,
    /// Secrets this action needs. How they are consumed is up to the selected
    /// instruction: shell injects env vars, a `uses` step passes `with:` inputs, etc.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<Ustr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actor_constraints: Vec<ActorConstraint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<ShellAction>,
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
}

impl Default for Policies {
    fn default() -> Self {
        Self { max_parallel_jobs: None, objectives: default_objectives() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub name: Ustr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub placements: Vec<Placement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<Actor>,
    #[serde(default)]
    pub policies: Policies,
}

impl Workflow {
    #[inline]
    pub fn artifact(&self, id: ArtifactId) -> &Artifact { &self.artifacts[id.idx()] }
    #[inline]
    pub fn action(&self, id: ActionId) -> &Action { &self.actions[id.idx()] }
    #[inline]
    pub fn effect(&self, id: EffectId) -> &Effect { &self.effects[id.idx()] }
    #[inline]
    pub fn actor(&self, id: ActorId) -> &Actor { &self.actors[id.idx()] }
}

#[derive(Default)]
pub struct WorkflowBuilder {
    name: Ustr,
    artifacts: Vec<Artifact>,
    actions: Vec<Action>,
    effects: Vec<Effect>,
    placements: Vec<Placement>,
    actors: Vec<Actor>,
    policies: Policies,
}

impl WorkflowBuilder {
    pub fn new(name: &str) -> Self {
        Self { name: name.into(), ..Default::default() }
    }

    /// Declare an artifact; returns its ID.
    pub fn artifact(&mut self, name: &str, ty: ArtifactType) -> ArtifactId {
        let id = ArtifactId(self.artifacts.len() as u32);
        self.artifacts.push(Artifact { name: name.into(), ty, producer: None, path: None });
        id
    }

    /// Declare an artifact with a workspace path; returns its ID.
    pub fn artifact_at(&mut self, name: &str, ty: ArtifactType, path: &str) -> ArtifactId {
        let id = ArtifactId(self.artifacts.len() as u32);
        self.artifacts.push(Artifact { name: name.into(), ty, producer: None, path: Some(path.into()) });
        id
    }

    /// Declare an action. Sets `producer` on each output artifact automatically.
    pub fn action(
        &mut self,
        name: &str,
        op: &str,
        inputs: &[ArtifactId],
        outputs: &[ArtifactId],
    ) -> ActionId {
        let id = ActionId(self.actions.len() as u32);
        self.actions.push(Action {
            name: name.into(),
            op: op.into(),
            inputs: inputs.to_vec(),
            outputs: outputs.to_vec(),
            effects: vec![],
            secrets: vec![],
            actor_constraints: vec![],
            shell: None,
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
        op: &str,
        inputs: &[ArtifactId],
        outputs: &[ArtifactId],
        script: &str,
    ) -> ActionId {
        let id = self.action(name, op, inputs, outputs);
        self.actions[id.idx()].shell = Some(ShellAction {
            script: script.into(),
            env: HashMap::new(),
            capture: CaptureRule::NoCapture,
        });
        id
    }

    pub fn add_effect_to(&mut self, action: ActionId, effect: EffectId) {
        self.actions[action.idx()].effects.push(effect);
    }

    /// Declare secrets an action needs, by name.
    pub fn add_secrets(&mut self, action: ActionId, names: &[&str]) {
        self.actions[action.idx()].secrets
            .extend(names.iter().map(|s| Ustr::from(s)));
    }

    /// Pin an action to a specific actor.
    pub fn constrain_actor(&mut self, action: ActionId, actor: ActorId) {
        self.actions[action.idx()].actor_constraints.push(ActorConstraint::Specific(actor));
    }

    /// Constrain an action to actors carrying a label (a soft constraint: the
    /// optimizer may pick any actor with this label).
    pub fn constrain_label(&mut self, action: ActionId, label: &str) {
        self.actions[action.idx()].actor_constraints.push(ActorConstraint::Label(Ustr::from(label)));
    }

    /// Declare a placement strategy for an artifact.
    pub fn place(&mut self, artifact: ArtifactId, strategy: PlacementStrategy) {
        self.placements.push(Placement { artifact, strategy });
    }

    pub fn effect(&mut self, name: &str, kind: EffectKind, requires_approval: bool) -> EffectId {
        let id = EffectId(self.effects.len() as u32);
        self.effects.push(Effect { name: name.into(), kind, requires_approval });
        id
    }

    pub fn actor(&mut self, name: &str, labels: &[&str], capabilities: &[&str]) -> ActorId {
        let id = ActorId(self.actors.len() as u32);
        self.actors.push(Actor {
            name: name.into(),
            labels: labels.iter().map(|s| Ustr::from(s)).collect(),
            capabilities: capabilities.iter().map(|s| Ustr::from(s)).collect(),
        });
        id
    }

    pub fn max_parallel_jobs(&mut self, n: usize) -> &mut Self {
        self.policies.max_parallel_jobs = Some(n);
        self
    }

    pub fn build(self) -> Workflow {
        Workflow {
            name: self.name,
            artifacts: self.artifacts,
            actions: self.actions,
            effects: self.effects,
            placements: self.placements,
            actors: self.actors,
            policies: self.policies,
        }
    }
}

//! TIR serialization. The Rust IR types (`crate::ir`) are the source of truth.
//! JSON is serde over those types; protobuf binary goes through the prost wire
//! types in [`wire`], which are defined in Rust.

use crate::ir;
use prost::Message;
use std::path::Path;
use ustr::Ustr;

/// Protobuf wire types. Hand-written prost messages, mirroring the IR. These
/// define the binary wire format; .
pub mod wire {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Workflow {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(message, repeated, tag = "2")]
        pub artifacts: Vec<Artifact>,
        #[prost(message, repeated, tag = "3")]
        pub action_calls: Vec<ActionCall>,
        #[prost(message, repeated, tag = "4")]
        pub consequences: Vec<Consequence>,
        #[prost(message, repeated, tag = "5")]
        pub placements: Vec<Placement>,
        #[prost(message, optional, tag = "7")]
        pub policies: Option<Policies>,
        #[prost(message, optional, tag = "8")]
        pub inventory: Option<Inventory>,
        #[prost(message, repeated, tag = "9")]
        pub triggers: Vec<Trigger>,
        #[prost(message, optional, tag = "10")]
        pub coordination: Option<Coordination>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Inventory {
        #[prost(string, tag = "1")]
        pub id: String,
        #[prost(message, repeated, tag = "2")]
        pub actors: Vec<Actor>,
        #[prost(message, repeated, tag = "3")]
        pub placements: Vec<InventoryPlacement>,
        #[prost(message, repeated, tag = "4")]
        pub implementations: Vec<Implementation>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Implementation {
        #[prost(string, tag = "1")]
        pub id: String,
        #[prost(string, optional, tag = "2")]
        pub version: Option<String>,
        #[prost(bool, tag = "3")]
        pub prefer: bool,
        #[prost(bool, tag = "4")]
        pub deny: bool,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct InventoryPlacement {
        #[prost(string, tag = "1")]
        pub id: String,
        #[prost(string, tag = "2")]
        pub kind: String,
        #[prost(string, repeated, tag = "3")]
        pub access_modes: Vec<String>,
        #[prost(string, repeated, tag = "4")]
        pub accessible_by: Vec<String>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Artifact {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(message, optional, tag = "2")]
        pub ty: Option<ArtifactType>,
        #[prost(uint32, optional, tag = "3")]
        pub producer: Option<u32>,
        #[prost(string, optional, tag = "4")]
        pub path: Option<String>,
        #[prost(string, optional, tag = "5")]
        pub lifetime: Option<String>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct ArtifactType {
        #[prost(oneof = "artifact_type::Kind", tags = "1, 2")]
        pub kind: Option<artifact_type::Kind>,
    }
    pub mod artifact_type {
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum Kind {
            #[prost(enumeration = "super::BuiltinArtifactType", tag = "1")]
            Builtin(i32),
            #[prost(string, tag = "2")]
            Custom(String),
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum BuiltinArtifactType {
        Unspecified = 0,
        SourceTree = 1,
        Wheel = 2,
        Binary = 3,
        ContainerImage = 4,
        Sbom = 5,
        Signature = 6,
        ReleaseBundle = 7,
        TestReport = 8,
        Model = 9,
        CoverageData = 10,
        DocsSite = 11,
        ProfileData = 12,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Consequence {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(message, optional, tag = "2")]
        pub kind: Option<ConsequenceKind>,
        #[prost(bool, tag = "3")]
        pub requires_approval: bool,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct ConsequenceKind {
        #[prost(oneof = "effect_kind::Kind", tags = "1, 2")]
        pub kind: Option<effect_kind::Kind>,
    }
    pub mod effect_kind {
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum Kind {
            #[prost(enumeration = "super::BuiltinConsequenceKind", tag = "1")]
            Builtin(i32),
            #[prost(string, tag = "2")]
            Custom(String),
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum BuiltinConsequenceKind {
        Unspecified = 0,
        Network = 1,
        SecretAccess = 2,
        GitWrite = 3,
        PublishRelease = 4,
        Deployment = 5,
        CommentOnPr = 6,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Actor {
        #[prost(string, tag = "1")]
        pub id: String,
        #[prost(string, repeated, tag = "2")]
        pub labels: Vec<String>,
        #[prost(string, repeated, tag = "3")]
        pub capabilities: Vec<String>,
        #[prost(message, optional, tag = "4")]
        pub resources: Option<Resources>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Placement {
        #[prost(uint32, tag = "1")]
        pub artifact: u32,
        #[prost(message, optional, tag = "2")]
        pub strategy: Option<PlacementStrategy>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct PlacementStrategy {
        #[prost(oneof = "placement_strategy::Strategy", tags = "1, 2, 3, 4, 5")]
        pub strategy: Option<placement_strategy::Strategy>,
    }
    pub mod placement_strategy {
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum Strategy {
            #[prost(message, tag = "1")]
            GithubArtifact(super::Unit),
            #[prost(message, tag = "2")]
            SharedVolume(super::SharedVolume),
            #[prost(message, tag = "3")]
            PersistentCache(super::PersistentCache),
            #[prost(message, tag = "4")]
            LocalPath(super::LocalPath),
            #[prost(message, tag = "5")]
            OciRegistry(super::OciRegistry),
        }
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Unit {}
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct SharedVolume {
        #[prost(string, tag = "1")]
        pub path: String,
    }
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct PersistentCache {
        #[prost(string, tag = "1")]
        pub key: String,
    }
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct LocalPath {
        #[prost(string, tag = "1")]
        pub path: String,
    }
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct OciRegistry {
        #[prost(string, tag = "1")]
        pub registry: String,
        #[prost(string, tag = "2")]
        pub tag: String,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct ActorConstraint {
        #[prost(oneof = "actor_constraint::Constraint", tags = "1, 2")]
        pub constraint: Option<actor_constraint::Constraint>,
    }
    pub mod actor_constraint {
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum Constraint {
            #[prost(uint32, tag = "1")]
            Specific(u32),
            #[prost(string, tag = "2")]
            Label(String),
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum CaptureRule {
        Unspecified = 0,
        NoCapture = 1,
        Stdout = 2,
        Stderr = 3,
        All = 4,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct ShellAction {
        #[prost(string, tag = "1")]
        pub script: String,
        #[prost(map = "string, string", tag = "2")]
        pub env: ::std::collections::HashMap<String, String>,
        #[prost(enumeration = "CaptureRule", tag = "3")]
        pub capture: i32,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct ActionCall {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(string, tag = "2")]
        pub op: String,
        #[prost(uint32, repeated, tag = "3")]
        pub inputs: Vec<u32>,
        #[prost(uint32, repeated, tag = "4")]
        pub outputs: Vec<u32>,
        #[prost(uint32, repeated, tag = "5")]
        pub consequences: Vec<u32>,
        #[prost(string, repeated, tag = "6")]
        pub secrets: Vec<String>,
        #[prost(message, repeated, tag = "7")]
        pub actor_constraints: Vec<ActorConstraint>,
        #[prost(message, optional, tag = "8")]
        pub shell: Option<ShellAction>,
        #[prost(string, optional, tag = "9")]
        pub timeout: Option<String>,
        #[prost(message, optional, tag = "10")]
        pub coordination: Option<Coordination>,
        #[prost(message, optional, tag = "11")]
        pub resources: Option<Resources>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Trigger {
        #[prost(oneof = "trigger::Kind", tags = "1, 2, 3, 4, 5")]
        pub kind: Option<trigger::Kind>,
    }
    pub mod trigger {
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum Kind {
            #[prost(message, tag = "1")]
            PullRequest(super::Unit),
            #[prost(message, tag = "2")]
            Push(super::PushTrigger),
            #[prost(string, tag = "3")]
            Tag(String),
            #[prost(string, tag = "4")]
            Schedule(String),
            #[prost(message, tag = "5")]
            Manual(super::Unit),
        }
    }
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct PushTrigger {
        #[prost(string, repeated, tag = "1")]
        pub branches: Vec<String>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Coordination {
        #[prost(string, tag = "1")]
        pub group: String,
        #[prost(bool, tag = "2")]
        pub cancel_in_progress: bool,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Resources {
        #[prost(uint32, optional, tag = "1")]
        pub cpu: Option<u32>,
        #[prost(string, optional, tag = "2")]
        pub memory: Option<String>,
        #[prost(string, optional, tag = "3")]
        pub disk: Option<String>,
        #[prost(uint32, optional, tag = "4")]
        pub gpu: Option<u32>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Policies {
        #[prost(uint32, optional, tag = "1")]
        pub max_parallel_jobs: Option<u32>,
        #[prost(enumeration = "Objective", repeated, tag = "2")]
        pub objectives: Vec<i32>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum Objective {
        Unspecified = 0,
        CriticalPath = 1,
        TransferBytes = 2,
        DollarCost = 3,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Binary,
    Json,
}

impl Format {
    pub fn from_extension(path: &Path) -> Option<Format> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("pb") => Some(Format::Binary),
            Some("json") => Some(Format::Json),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum CodecError {
    Decode(String),
    Json(String),
    Io(String),
    UnknownExtension(String),
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodecError::Decode(e) => write!(f, "protobuf decode error: {e}"),
            CodecError::Json(e) => write!(f, "json error: {e}"),
            CodecError::Io(e) => write!(f, "io error: {e}"),
            CodecError::UnknownExtension(p) => {
                write!(f, "unknown TIR extension for '{p}' (expected .pb/.json)")
            }
        }
    }
}
impl std::error::Error for CodecError {}

pub fn encode(wf: &ir::Workflow, fmt: Format) -> Result<Vec<u8>, CodecError> {
    match fmt {
        Format::Binary => Ok(to_wire(wf).encode_to_vec()),
        Format::Json => serde_json::to_vec_pretty(wf).map_err(|e| CodecError::Json(e.to_string())),
    }
}

pub fn decode(bytes: &[u8], fmt: Format) -> Result<ir::Workflow, CodecError> {
    match fmt {
        Format::Binary => {
            let w = wire::Workflow::decode(bytes).map_err(|e| CodecError::Decode(e.to_string()))?;
            Ok(from_wire(&w))
        }
        Format::Json => serde_json::from_slice(bytes).map_err(|e| CodecError::Json(e.to_string())),
    }
}

pub fn load(path: &Path) -> Result<ir::Workflow, CodecError> {
    let fmt = Format::from_extension(path)
        .ok_or_else(|| CodecError::UnknownExtension(path.display().to_string()))?;
    let bytes = std::fs::read(path).map_err(|e| CodecError::Io(e.to_string()))?;
    decode(&bytes, fmt)
}

pub fn save(path: &Path, wf: &ir::Workflow) -> Result<(), CodecError> {
    let fmt = Format::from_extension(path)
        .ok_or_else(|| CodecError::UnknownExtension(path.display().to_string()))?;
    let bytes = encode(wf, fmt)?;
    std::fs::write(path, bytes).map_err(|e| CodecError::Io(e.to_string()))
}

pub fn to_wire(wf: &ir::Workflow) -> wire::Workflow {
    wire::Workflow {
        name: wf.name.to_string(),
        artifacts: wf.artifacts.iter().map(artifact_to).collect(),
        action_calls: wf.action_calls.iter().map(action_to).collect(),
        consequences: wf.consequences.iter().map(effect_to).collect(),
        placements: wf.placements.iter().map(placement_to).collect(),
        policies: Some(wire::Policies {
            max_parallel_jobs: wf.policies.max_parallel_jobs.map(|n| n as u32),
            objectives: wf
                .policies
                .objectives
                .iter()
                .map(|o| objective_to(o) as i32)
                .collect(),
        }),
        inventory: wf.inventory.as_ref().map(inventory_to),
        triggers: wf.triggers.iter().map(trigger_to).collect(),
        coordination: wf.coordination.as_ref().map(coordination_to),
    }
}

pub fn from_wire(m: &wire::Workflow) -> ir::Workflow {
    ir::Workflow {
        name: Ustr::from(m.name.as_str()),
        artifacts: m.artifacts.iter().map(artifact_from).collect(),
        action_calls: m.action_calls.iter().map(action_from).collect(),
        consequences: m.consequences.iter().map(effect_from).collect(),
        placements: m.placements.iter().map(placement_from).collect(),
        inventory: m.inventory.as_ref().map(inventory_from),
        policies: ir::Policies {
            max_parallel_jobs: m
                .policies
                .as_ref()
                .and_then(|p| p.max_parallel_jobs)
                .map(|n| n as usize),
            objectives: {
                let objs: Vec<ir::Objective> = m
                    .policies
                    .as_ref()
                    .map(|p| {
                        p.objectives
                            .iter()
                            .filter_map(|&i| objective_from(i))
                            .collect()
                    })
                    .unwrap_or_default();
                if objs.is_empty() {
                    ir::default_objectives()
                } else {
                    objs
                }
            },
            // Not carried on the binary wire; JSON TIR carries it via serde.
            install_dependencies: false,
        },
        // ActionDefs are not carried on the binary wire (they are a frontend
        // authoring detail; the planner uses the inlined action fields).
        action_defs: vec![],
        triggers: m.triggers.iter().filter_map(trigger_from).collect(),
        coordination: m.coordination.as_ref().map(coordination_from),
    }
}

fn inventory_to(inv: &ir::Inventory) -> wire::Inventory {
    wire::Inventory {
        id: inv.id.to_string(),
        actors: inv.actors.iter().map(actor_to).collect(),
        placements: inv.placements.iter().map(inv_placement_to).collect(),
        implementations: inv.implementations.iter().map(impl_to).collect(),
    }
}
fn inventory_from(m: &wire::Inventory) -> ir::Inventory {
    ir::Inventory {
        id: Ustr::from(m.id.as_str()),
        actors: m.actors.iter().map(actor_from).collect(),
        placements: m.placements.iter().map(inv_placement_from).collect(),
        implementations: m.implementations.iter().map(impl_from).collect(),
    }
}
fn impl_to(i: &ir::InventoryImpl) -> wire::Implementation {
    wire::Implementation {
        id: i.id.to_string(),
        version: i.version.as_ref().map(|v| v.to_string()),
        prefer: i.prefer,
        deny: i.deny,
    }
}
fn impl_from(m: &wire::Implementation) -> ir::InventoryImpl {
    ir::InventoryImpl {
        id: Ustr::from(m.id.as_str()),
        version: m.version.as_deref().map(Ustr::from),
        prefer: m.prefer,
        deny: m.deny,
    }
}
fn inv_placement_to(p: &ir::InventoryPlacement) -> wire::InventoryPlacement {
    wire::InventoryPlacement {
        id: p.id.to_string(),
        kind: p.kind.to_string(),
        access_modes: p.access_modes.iter().map(|s| s.to_string()).collect(),
        accessible_by: p.accessible_by.iter().map(|s| s.to_string()).collect(),
    }
}
fn inv_placement_from(m: &wire::InventoryPlacement) -> ir::InventoryPlacement {
    ir::InventoryPlacement {
        id: Ustr::from(m.id.as_str()),
        kind: Ustr::from(m.kind.as_str()),
        access_modes: m
            .access_modes
            .iter()
            .map(|s| Ustr::from(s.as_str()))
            .collect(),
        accessible_by: m
            .accessible_by
            .iter()
            .map(|s| Ustr::from(s.as_str()))
            .collect(),
    }
}

fn objective_to(o: &ir::Objective) -> wire::Objective {
    match o {
        ir::Objective::CriticalPath => wire::Objective::CriticalPath,
        ir::Objective::TransferBytes => wire::Objective::TransferBytes,
        ir::Objective::DollarCost => wire::Objective::DollarCost,
    }
}
fn objective_from(i: i32) -> Option<ir::Objective> {
    match wire::Objective::try_from(i).ok()? {
        wire::Objective::CriticalPath => Some(ir::Objective::CriticalPath),
        wire::Objective::TransferBytes => Some(ir::Objective::TransferBytes),
        wire::Objective::DollarCost => Some(ir::Objective::DollarCost),
        wire::Objective::Unspecified => None,
    }
}

fn artifact_to(a: &ir::Artifact) -> wire::Artifact {
    wire::Artifact {
        name: a.name.to_string(),
        ty: Some(artifact_type_to(&a.ty)),
        producer: a.producer.map(|id| id.0),
        path: a.path.clone(),
        lifetime: a.lifetime.clone(),
    }
}
fn artifact_from(m: &wire::Artifact) -> ir::Artifact {
    ir::Artifact {
        name: ustr::Ustr::from(m.name.as_str()),
        ty: m
            .ty
            .as_ref()
            .map(artifact_type_from)
            .unwrap_or(ir::ArtifactType::SourceTree),
        producer: m.producer.map(ir::ActionCallId),
        path: m.path.clone(),
        lifetime: m.lifetime.clone(),
    }
}

fn artifact_type_to(t: &ir::ArtifactType) -> wire::ArtifactType {
    use ir::ArtifactType as A;
    use wire::BuiltinArtifactType as B;
    use wire::artifact_type::Kind;
    let kind = match t {
        A::Custom(s) => Kind::Custom(s.clone()),
        A::SourceTree => Kind::Builtin(B::SourceTree as i32),
        A::Wheel => Kind::Builtin(B::Wheel as i32),
        A::Binary => Kind::Builtin(B::Binary as i32),
        A::ContainerImage => Kind::Builtin(B::ContainerImage as i32),
        A::Sbom => Kind::Builtin(B::Sbom as i32),
        A::Signature => Kind::Builtin(B::Signature as i32),
        A::ReleaseBundle => Kind::Builtin(B::ReleaseBundle as i32),
        A::TestReport => Kind::Builtin(B::TestReport as i32),
        A::CoverageData => Kind::Builtin(B::CoverageData as i32),
        A::DocsSite => Kind::Builtin(B::DocsSite as i32),
        A::ProfileData => Kind::Builtin(B::ProfileData as i32),
        A::Model => Kind::Builtin(B::Model as i32),
    };
    wire::ArtifactType { kind: Some(kind) }
}
fn artifact_type_from(m: &wire::ArtifactType) -> ir::ArtifactType {
    use ir::ArtifactType as A;
    use wire::BuiltinArtifactType as B;
    use wire::artifact_type::Kind;
    match &m.kind {
        Some(Kind::Custom(s)) => A::Custom(s.clone()),
        Some(Kind::Builtin(i)) => match B::try_from(*i).unwrap_or(B::Unspecified) {
            B::Wheel => A::Wheel,
            B::Binary => A::Binary,
            B::ContainerImage => A::ContainerImage,
            B::Sbom => A::Sbom,
            B::Signature => A::Signature,
            B::ReleaseBundle => A::ReleaseBundle,
            B::TestReport => A::TestReport,
            B::CoverageData => A::CoverageData,
            B::DocsSite => A::DocsSite,
            B::ProfileData => A::ProfileData,
            B::Model => A::Model,
            _ => A::SourceTree,
        },
        None => A::SourceTree,
    }
}

fn effect_to(e: &ir::Consequence) -> wire::Consequence {
    wire::Consequence {
        name: e.name.to_string(),
        kind: Some(effect_kind_to(&e.kind)),
        requires_approval: e.requires_approval,
    }
}
fn effect_from(m: &wire::Consequence) -> ir::Consequence {
    ir::Consequence {
        name: Ustr::from(m.name.as_str()),
        kind: m
            .kind
            .as_ref()
            .map(effect_kind_from)
            .unwrap_or(ir::ConsequenceKind::Network),
        requires_approval: m.requires_approval,
    }
}
fn effect_kind_to(k: &ir::ConsequenceKind) -> wire::ConsequenceKind {
    use ir::ConsequenceKind as E;
    use wire::BuiltinConsequenceKind as B;
    use wire::effect_kind::Kind;
    let kind = match k {
        E::Custom(s) => Kind::Custom(s.clone()),
        E::Network => Kind::Builtin(B::Network as i32),
        E::SecretAccess => Kind::Builtin(B::SecretAccess as i32),
        E::GitWrite => Kind::Builtin(B::GitWrite as i32),
        E::PublishRelease => Kind::Builtin(B::PublishRelease as i32),
        E::Deployment => Kind::Builtin(B::Deployment as i32),
        E::CommentOnPr => Kind::Builtin(B::CommentOnPr as i32),
    };
    wire::ConsequenceKind { kind: Some(kind) }
}
fn effect_kind_from(m: &wire::ConsequenceKind) -> ir::ConsequenceKind {
    use ir::ConsequenceKind as E;
    use wire::BuiltinConsequenceKind as B;
    use wire::effect_kind::Kind;
    match &m.kind {
        Some(Kind::Custom(s)) => E::Custom(s.clone()),
        Some(Kind::Builtin(i)) => match B::try_from(*i).unwrap_or(B::Unspecified) {
            B::SecretAccess => E::SecretAccess,
            B::GitWrite => E::GitWrite,
            B::PublishRelease => E::PublishRelease,
            B::Deployment => E::Deployment,
            B::CommentOnPr => E::CommentOnPr,
            _ => E::Network,
        },
        None => E::Network,
    }
}

fn actor_to(a: &ir::Actor) -> wire::Actor {
    wire::Actor {
        id: a.id.to_string(),
        labels: a.labels.iter().map(|s| s.to_string()).collect(),
        capabilities: a.capabilities.iter().map(|s| s.to_string()).collect(),
        resources: a.resources.as_ref().map(resources_to),
    }
}
fn actor_from(m: &wire::Actor) -> ir::Actor {
    ir::Actor {
        id: Ustr::from(m.id.as_str()),
        labels: m.labels.iter().map(|s| Ustr::from(s.as_str())).collect(),
        capabilities: m
            .capabilities
            .iter()
            .map(|s| Ustr::from(s.as_str()))
            .collect(),
        resources: m.resources.as_ref().map(resources_from),
    }
}

fn placement_to(p: &ir::Placement) -> wire::Placement {
    wire::Placement {
        artifact: p.artifact.0,
        strategy: Some(placement_strategy_to(&p.strategy)),
    }
}
fn placement_from(m: &wire::Placement) -> ir::Placement {
    ir::Placement {
        artifact: ir::ArtifactId(m.artifact),
        strategy: m
            .strategy
            .as_ref()
            .map(placement_strategy_from)
            .unwrap_or(ir::PlacementStrategy::GithubArtifact),
    }
}
fn placement_strategy_to(s: &ir::PlacementStrategy) -> wire::PlacementStrategy {
    use ir::PlacementStrategy as P;
    use wire::placement_strategy::Strategy;
    let strategy = match s {
        P::GithubArtifact => Strategy::GithubArtifact(wire::Unit {}),
        P::SharedVolume { path } => {
            Strategy::SharedVolume(wire::SharedVolume { path: path.clone() })
        }
        P::PersistentCache { key } => {
            Strategy::PersistentCache(wire::PersistentCache { key: key.clone() })
        }
        P::LocalPath { path } => Strategy::LocalPath(wire::LocalPath { path: path.clone() }),
        P::OciRegistry { registry, tag } => Strategy::OciRegistry(wire::OciRegistry {
            registry: registry.clone(),
            tag: tag.clone(),
        }),
    };
    wire::PlacementStrategy {
        strategy: Some(strategy),
    }
}
fn placement_strategy_from(m: &wire::PlacementStrategy) -> ir::PlacementStrategy {
    use ir::PlacementStrategy as P;
    use wire::placement_strategy::Strategy;
    match &m.strategy {
        Some(Strategy::SharedVolume(v)) => P::SharedVolume {
            path: v.path.clone(),
        },
        Some(Strategy::PersistentCache(v)) => P::PersistentCache { key: v.key.clone() },
        Some(Strategy::LocalPath(v)) => P::LocalPath {
            path: v.path.clone(),
        },
        Some(Strategy::OciRegistry(v)) => P::OciRegistry {
            registry: v.registry.clone(),
            tag: v.tag.clone(),
        },
        _ => P::GithubArtifact,
    }
}

fn action_to(a: &ir::ActionCall) -> wire::ActionCall {
    wire::ActionCall {
        name: a.name.to_string(),
        op: a.action.to_string(),
        inputs: a.inputs.iter().map(|id| id.0).collect(),
        outputs: a.outputs.iter().map(|id| id.0).collect(),
        consequences: a.consequences.iter().map(|id| id.0).collect(),
        secrets: a.secrets.iter().map(|s| s.to_string()).collect(),
        actor_constraints: a
            .actor_constraints
            .iter()
            .map(actor_constraint_to)
            .collect(),
        shell: a.shell.as_ref().map(shell_to),
        timeout: a.timeout.clone(),
        coordination: a.coordination.as_ref().map(coordination_to),
        resources: a.resources.as_ref().map(resources_to),
    }
}
fn action_from(m: &wire::ActionCall) -> ir::ActionCall {
    ir::ActionCall {
        name: Ustr::from(m.name.as_str()),
        action: Ustr::from(m.op.as_str()),
        inputs: m.inputs.iter().map(|&i| ir::ArtifactId(i)).collect(),
        outputs: m.outputs.iter().map(|&i| ir::ArtifactId(i)).collect(),
        // Explicit ordering edges are not carried on the binary wire; JSON TIR
        // carries them via serde.
        after: vec![],
        consequences: m
            .consequences
            .iter()
            .map(|&i| ir::ConsequenceId(i))
            .collect(),
        secrets: m.secrets.iter().map(|s| Ustr::from(s.as_str())).collect(),
        actor_constraints: m
            .actor_constraints
            .iter()
            .map(actor_constraint_from)
            .collect(),
        shell: m.shell.as_ref().map(shell_from),
        timeout: m.timeout.clone(),
        coordination: m.coordination.as_ref().map(coordination_from),
        resources: m.resources.as_ref().map(resources_from),
    }
}

fn trigger_to(t: &ir::Trigger) -> wire::Trigger {
    use wire::trigger::Kind;
    let kind = match t {
        ir::Trigger::PullRequest => Kind::PullRequest(wire::Unit {}),
        ir::Trigger::Push { branches } => Kind::Push(wire::PushTrigger {
            branches: branches.clone(),
        }),
        ir::Trigger::Tag { pattern } => Kind::Tag(pattern.clone()),
        ir::Trigger::Schedule { cron } => Kind::Schedule(cron.clone()),
        ir::Trigger::Manual => Kind::Manual(wire::Unit {}),
    };
    wire::Trigger { kind: Some(kind) }
}
fn trigger_from(m: &wire::Trigger) -> Option<ir::Trigger> {
    use wire::trigger::Kind;
    Some(match m.kind.as_ref()? {
        Kind::PullRequest(_) => ir::Trigger::PullRequest,
        Kind::Push(p) => ir::Trigger::Push {
            branches: p.branches.clone(),
        },
        Kind::Tag(pattern) => ir::Trigger::Tag {
            pattern: pattern.clone(),
        },
        Kind::Schedule(cron) => ir::Trigger::Schedule { cron: cron.clone() },
        Kind::Manual(_) => ir::Trigger::Manual,
    })
}

fn coordination_to(c: &ir::Coordination) -> wire::Coordination {
    wire::Coordination {
        group: c.group.clone(),
        cancel_in_progress: c.cancel_in_progress,
    }
}
fn coordination_from(m: &wire::Coordination) -> ir::Coordination {
    ir::Coordination {
        group: m.group.clone(),
        cancel_in_progress: m.cancel_in_progress,
    }
}

fn resources_to(r: &ir::Resources) -> wire::Resources {
    wire::Resources {
        cpu: r.cpu,
        memory: r.memory.clone(),
        disk: r.disk.clone(),
        gpu: r.gpu,
    }
}
fn resources_from(m: &wire::Resources) -> ir::Resources {
    ir::Resources {
        cpu: m.cpu,
        memory: m.memory.clone(),
        disk: m.disk.clone(),
        gpu: m.gpu,
    }
}

fn actor_constraint_to(c: &ir::ActorConstraint) -> wire::ActorConstraint {
    use wire::actor_constraint::Constraint;
    let constraint = match c {
        ir::ActorConstraint::Specific(id) => Constraint::Specific(id.0),
        ir::ActorConstraint::Label(l) => Constraint::Label(l.to_string()),
    };
    wire::ActorConstraint {
        constraint: Some(constraint),
    }
}
fn actor_constraint_from(m: &wire::ActorConstraint) -> ir::ActorConstraint {
    use wire::actor_constraint::Constraint;
    match &m.constraint {
        Some(Constraint::Specific(id)) => ir::ActorConstraint::Specific(ir::ActorId(*id)),
        Some(Constraint::Label(l)) => ir::ActorConstraint::Label(Ustr::from(l.as_str())),
        None => ir::ActorConstraint::Label(Ustr::from("")),
    }
}

fn shell_to(s: &ir::ShellAction) -> wire::ShellAction {
    wire::ShellAction {
        script: s.script.clone(),
        env: s.env.clone(),
        capture: capture_to(&s.capture) as i32,
    }
}
fn shell_from(m: &wire::ShellAction) -> ir::ShellAction {
    ir::ShellAction {
        script: m.script.clone(),
        env: m.env.clone(),
        capture: wire::CaptureRule::try_from(m.capture)
            .map(capture_from)
            .unwrap_or(ir::CaptureRule::NoCapture),
    }
}
fn capture_to(c: &ir::CaptureRule) -> wire::CaptureRule {
    use wire::CaptureRule as C;
    match c {
        ir::CaptureRule::NoCapture => C::NoCapture,
        ir::CaptureRule::Stdout => C::Stdout,
        ir::CaptureRule::Stderr => C::Stderr,
        ir::CaptureRule::All => C::All,
    }
}
fn capture_from(c: wire::CaptureRule) -> ir::CaptureRule {
    use wire::CaptureRule as C;
    match c {
        C::Stdout => ir::CaptureRule::Stdout,
        C::Stderr => ir::CaptureRule::Stderr,
        C::All => ir::CaptureRule::All,
        _ => ir::CaptureRule::NoCapture,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::*;

    fn rich() -> Workflow {
        let mut b = WorkflowBuilder::new("Roundtrip");
        let src = b.artifact("source", ArtifactType::SourceTree);
        let bin = b.artifact_at("binary", ArtifactType::Binary, "target/release/app");
        let custom = b.artifact("weird", ArtifactType::Custom("thing".into()));
        let checkout = b.shell_action("checkout", "checkout", &[], &[src], "git checkout .");
        let build = b.shell_action("build", "build", &[src], &[bin, custom], "make");
        b.add_secrets(build, &["TOKEN", "KEY"]);
        let eff = b.consequence("deploy", ConsequenceKind::Deployment, true);
        b.add_consequence_to(build, eff);
        b.consequence("fx", ConsequenceKind::Custom("weird-fx".into()), false);
        let actor = b.actor("big", &["self-hosted"], &["mount", "gpu"]);
        b.constrain_actor(checkout, actor);
        b.place(
            src,
            PlacementStrategy::SharedVolume {
                path: "/vol".into(),
            },
        );
        b.place(
            bin,
            PlacementStrategy::OciRegistry {
                registry: "ghcr.io".into(),
                tag: "v1".into(),
            },
        );
        b.max_parallel_jobs(4);
        b.implementation("git", None, false, false);
        b.implementation("maturin", Some("1.7"), true, false);
        b.implementation("docker", None, false, true);
        let mut wf = b.build();
        wf.triggers = vec![
            Trigger::Push {
                branches: vec!["main".into()],
            },
            Trigger::Tag {
                pattern: "v*".into(),
            },
            Trigger::Manual,
        ];
        wf.coordination = Some(Coordination {
            group: "release".into(),
            cancel_in_progress: true,
        });
        wf.artifacts[bin.idx()].lifetime = Some("14d".into());
        wf.action_calls[build.idx()].timeout = Some("30m".into());
        wf.action_calls[build.idx()].coordination = Some(Coordination {
            group: "prod".into(),
            cancel_in_progress: false,
        });
        wf.action_calls[build.idx()].resources = Some(Resources {
            cpu: Some(8),
            memory: Some("32Gi".into()),
            disk: None,
            gpu: Some(1),
        });
        wf.inventory.as_mut().unwrap().actors[0].resources = Some(Resources {
            cpu: Some(16),
            ..Default::default()
        });
        wf
    }

    fn check(back: &Workflow) {
        assert_eq!(back.artifacts[2].ty, ArtifactType::Custom("thing".into()));
        assert_eq!(back.artifacts[1].producer, Some(ActionCallId(1)));
        assert_eq!(
            back.artifacts[1].path.as_deref(),
            Some("target/release/app")
        );
        assert_eq!(
            back.action_calls[1].secrets,
            vec!["TOKEN".to_string(), "KEY".to_string()]
        );
        assert!(matches!(
            back.action_calls[0].actor_constraints[0],
            ActorConstraint::Specific(ActorId(0))
        ));
        assert_eq!(back.consequences[0].kind, ConsequenceKind::Deployment);
        assert!(back.consequences[0].requires_approval);
        assert_eq!(
            back.consequences[1].kind,
            ConsequenceKind::Custom("weird-fx".into())
        );
        assert_eq!(
            back.inventory.as_ref().unwrap().actors[0].capabilities,
            vec!["mount".to_string(), "gpu".to_string()]
        );
        assert!(matches!(
            back.placements[1].strategy,
            PlacementStrategy::OciRegistry { .. }
        ));
        assert_eq!(back.policies.max_parallel_jobs, Some(4));
        let impls = &back.inventory.as_ref().unwrap().implementations;
        assert_eq!(impls[0].id.as_str(), "git");
        assert_eq!(impls[1].id.as_str(), "maturin");
        assert_eq!(impls[1].version.as_deref(), Some("1.7"));
        assert!(impls[1].prefer);
        assert!(impls[2].deny);
        // Execution semantics round-trip.
        assert_eq!(back.triggers.len(), 3);
        assert!(matches!(back.triggers[0], Trigger::Push { .. }));
        assert!(matches!(back.triggers[1], Trigger::Tag { .. }));
        assert!(matches!(back.triggers[2], Trigger::Manual));
        assert_eq!(back.coordination.as_ref().unwrap().group, "release");
        assert!(back.coordination.as_ref().unwrap().cancel_in_progress);
        assert_eq!(back.artifacts[1].lifetime.as_deref(), Some("14d"));
        assert_eq!(back.action_calls[1].timeout.as_deref(), Some("30m"));
        assert_eq!(
            back.action_calls[1].coordination.as_ref().unwrap().group,
            "prod"
        );
        let res = back.action_calls[1].resources.as_ref().unwrap();
        assert_eq!(res.cpu, Some(8));
        assert_eq!(res.memory.as_deref(), Some("32Gi"));
        assert_eq!(res.gpu, Some(1));
        assert_eq!(
            back.inventory.as_ref().unwrap().actors[0]
                .resources
                .as_ref()
                .unwrap()
                .cpu,
            Some(16)
        );
    }

    #[test]
    fn roundtrip_binary() {
        let wf = rich();
        check(&decode(&encode(&wf, Format::Binary).unwrap(), Format::Binary).unwrap());
    }

    #[test]
    fn roundtrip_json() {
        let wf = rich();
        check(&decode(&encode(&wf, Format::Json).unwrap(), Format::Json).unwrap());
    }

    #[test]
    fn json_is_serde_of_ir() {
        let json = String::from_utf8(encode(&rich(), Format::Json).unwrap()).unwrap();
        assert!(json.contains("\"Binary\""));
        assert!(json.contains("\"max_parallel_jobs\": 4"));
    }

    #[test]
    fn format_by_extension() {
        assert_eq!(
            Format::from_extension(Path::new("a.pb")),
            Some(Format::Binary)
        );
        assert_eq!(
            Format::from_extension(Path::new("a.json")),
            Some(Format::Json)
        );
        assert_eq!(Format::from_extension(Path::new("a.txt")), None);
    }
}

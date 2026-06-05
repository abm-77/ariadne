//! TIR serialization. The Rust IR types (`crate::ir`) are the source of truth.
//! JSON is serde over those types; protobuf binary goes through the prost wire
//! types in [`wire`], which are defined in Rust.

use crate::ir;
use prost::Message;
use ustr::Ustr;
use std::path::Path;

/// Protobuf wire types. Hand-written prost messages, mirroring the IR. These
/// define the binary wire format; .
pub mod wire {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Workflow {
        #[prost(string, tag = "1")] pub name: String,
        #[prost(message, repeated, tag = "2")] pub artifacts: Vec<Artifact>,
        #[prost(message, repeated, tag = "3")] pub actions: Vec<Action>,
        #[prost(message, repeated, tag = "4")] pub effects: Vec<Effect>,
        #[prost(message, repeated, tag = "5")] pub placements: Vec<Placement>,
        #[prost(message, repeated, tag = "6")] pub actors: Vec<Actor>,
        #[prost(message, optional, tag = "7")] pub policies: Option<Policies>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Artifact {
        #[prost(string, tag = "1")] pub name: String,
        #[prost(message, optional, tag = "2")] pub ty: Option<ArtifactType>,
        #[prost(uint32, optional, tag = "3")] pub producer: Option<u32>,
        #[prost(string, optional, tag = "4")] pub path: Option<String>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct ArtifactType {
        #[prost(oneof = "artifact_type::Kind", tags = "1, 2")]
        pub kind: Option<artifact_type::Kind>,
    }
    pub mod artifact_type {
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum Kind {
            #[prost(enumeration = "super::BuiltinArtifactType", tag = "1")] Builtin(i32),
            #[prost(string, tag = "2")] Custom(String),
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
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Effect {
        #[prost(string, tag = "1")] pub name: String,
        #[prost(message, optional, tag = "2")] pub kind: Option<EffectKind>,
        #[prost(bool, tag = "3")] pub requires_approval: bool,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct EffectKind {
        #[prost(oneof = "effect_kind::Kind", tags = "1, 2")]
        pub kind: Option<effect_kind::Kind>,
    }
    pub mod effect_kind {
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum Kind {
            #[prost(enumeration = "super::BuiltinEffectKind", tag = "1")] Builtin(i32),
            #[prost(string, tag = "2")] Custom(String),
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum BuiltinEffectKind {
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
        #[prost(string, tag = "1")] pub name: String,
        #[prost(string, repeated, tag = "2")] pub labels: Vec<String>,
        #[prost(string, repeated, tag = "3")] pub capabilities: Vec<String>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Placement {
        #[prost(uint32, tag = "1")] pub artifact: u32,
        #[prost(message, optional, tag = "2")] pub strategy: Option<PlacementStrategy>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct PlacementStrategy {
        #[prost(oneof = "placement_strategy::Strategy", tags = "1, 2, 3, 4, 5")]
        pub strategy: Option<placement_strategy::Strategy>,
    }
    pub mod placement_strategy {
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum Strategy {
            #[prost(message, tag = "1")] GithubArtifact(super::Unit),
            #[prost(message, tag = "2")] SharedVolume(super::SharedVolume),
            #[prost(message, tag = "3")] PersistentCache(super::PersistentCache),
            #[prost(message, tag = "4")] LocalPath(super::LocalPath),
            #[prost(message, tag = "5")] OciRegistry(super::OciRegistry),
        }
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Unit {}
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct SharedVolume { #[prost(string, tag = "1")] pub path: String }
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct PersistentCache { #[prost(string, tag = "1")] pub key: String }
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct LocalPath { #[prost(string, tag = "1")] pub path: String }
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct OciRegistry {
        #[prost(string, tag = "1")] pub registry: String,
        #[prost(string, tag = "2")] pub tag: String,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct ActorConstraint {
        #[prost(oneof = "actor_constraint::Constraint", tags = "1, 2")]
        pub constraint: Option<actor_constraint::Constraint>,
    }
    pub mod actor_constraint {
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum Constraint {
            #[prost(uint32, tag = "1")] Specific(u32),
            #[prost(string, tag = "2")] Label(String),
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
        #[prost(string, tag = "1")] pub script: String,
        #[prost(map = "string, string", tag = "2")] pub env: ::std::collections::HashMap<String, String>,
        #[prost(enumeration = "CaptureRule", tag = "3")] pub capture: i32,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Action {
        #[prost(string, tag = "1")] pub name: String,
        #[prost(string, tag = "2")] pub op: String,
        #[prost(uint32, repeated, tag = "3")] pub inputs: Vec<u32>,
        #[prost(uint32, repeated, tag = "4")] pub outputs: Vec<u32>,
        #[prost(uint32, repeated, tag = "5")] pub effects: Vec<u32>,
        #[prost(string, repeated, tag = "6")] pub secrets: Vec<String>,
        #[prost(message, repeated, tag = "7")] pub actor_constraints: Vec<ActorConstraint>,
        #[prost(message, optional, tag = "8")] pub shell: Option<ShellAction>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Policies {
        #[prost(uint32, optional, tag = "1")] pub max_parallel_jobs: Option<u32>,
        #[prost(enumeration = "Objective", repeated, tag = "2")] pub objectives: Vec<i32>,
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
            CodecError::UnknownExtension(p) => write!(f, "unknown TIR extension for '{p}' (expected .pb/.json)"),
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
        actions: wf.actions.iter().map(action_to).collect(),
        effects: wf.effects.iter().map(effect_to).collect(),
        placements: wf.placements.iter().map(placement_to).collect(),
        actors: wf.actors.iter().map(actor_to).collect(),
        policies: Some(wire::Policies {
            max_parallel_jobs: wf.policies.max_parallel_jobs.map(|n| n as u32),
            objectives: wf.policies.objectives.iter().map(|o| objective_to(o) as i32).collect(),
        }),
    }
}

pub fn from_wire(m: &wire::Workflow) -> ir::Workflow {
    ir::Workflow {
        name: Ustr::from(m.name.as_str()),
        artifacts: m.artifacts.iter().map(artifact_from).collect(),
        actions: m.actions.iter().map(action_from).collect(),
        effects: m.effects.iter().map(effect_from).collect(),
        placements: m.placements.iter().map(placement_from).collect(),
        actors: m.actors.iter().map(actor_from).collect(),
        policies: ir::Policies {
            max_parallel_jobs: m.policies.as_ref().and_then(|p| p.max_parallel_jobs).map(|n| n as usize),
            objectives: {
                let objs: Vec<ir::Objective> = m.policies.as_ref()
                    .map(|p| p.objectives.iter().filter_map(|&i| objective_from(i)).collect())
                    .unwrap_or_default();
                if objs.is_empty() { ir::default_objectives() } else { objs }
            },
        },
        op_definitions: vec![],
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
    }
}
fn artifact_from(m: &wire::Artifact) -> ir::Artifact {
    ir::Artifact {
        name: ustr::Ustr::from(m.name.as_str()),
        ty: m.ty.as_ref().map(artifact_type_from).unwrap_or(ir::ArtifactType::SourceTree),
        producer: m.producer.map(ir::ActionId),
        path: m.path.clone(),
    }
}

fn artifact_type_to(t: &ir::ArtifactType) -> wire::ArtifactType {
    use wire::artifact_type::Kind;
    use wire::BuiltinArtifactType as B;
    use ir::ArtifactType as A;
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
        A::Model => Kind::Builtin(B::Model as i32),
    };
    wire::ArtifactType { kind: Some(kind) }
}
fn artifact_type_from(m: &wire::ArtifactType) -> ir::ArtifactType {
    use wire::artifact_type::Kind;
    use wire::BuiltinArtifactType as B;
    use ir::ArtifactType as A;
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
            B::Model => A::Model,
            _ => A::SourceTree,
        },
        None => A::SourceTree,
    }
}

fn effect_to(e: &ir::Effect) -> wire::Effect {
    wire::Effect { name: e.name.to_string(), kind: Some(effect_kind_to(&e.kind)), requires_approval: e.requires_approval }
}
fn effect_from(m: &wire::Effect) -> ir::Effect {
    ir::Effect {
        name: Ustr::from(m.name.as_str()),
        kind: m.kind.as_ref().map(effect_kind_from).unwrap_or(ir::EffectKind::Network),
        requires_approval: m.requires_approval,
    }
}
fn effect_kind_to(k: &ir::EffectKind) -> wire::EffectKind {
    use wire::effect_kind::Kind;
    use wire::BuiltinEffectKind as B;
    use ir::EffectKind as E;
    let kind = match k {
        E::Custom(s) => Kind::Custom(s.clone()),
        E::Network => Kind::Builtin(B::Network as i32),
        E::SecretAccess => Kind::Builtin(B::SecretAccess as i32),
        E::GitWrite => Kind::Builtin(B::GitWrite as i32),
        E::PublishRelease => Kind::Builtin(B::PublishRelease as i32),
        E::Deployment => Kind::Builtin(B::Deployment as i32),
        E::CommentOnPr => Kind::Builtin(B::CommentOnPr as i32),
    };
    wire::EffectKind { kind: Some(kind) }
}
fn effect_kind_from(m: &wire::EffectKind) -> ir::EffectKind {
    use wire::effect_kind::Kind;
    use wire::BuiltinEffectKind as B;
    use ir::EffectKind as E;
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
    wire::Actor { name: a.name.to_string(), labels: a.labels.iter().map(|s| s.to_string()).collect(), capabilities: a.capabilities.iter().map(|s| s.to_string()).collect() }
}
fn actor_from(m: &wire::Actor) -> ir::Actor {
    ir::Actor { name: Ustr::from(m.name.as_str()), labels: m.labels.iter().map(|s| Ustr::from(s.as_str())).collect(), capabilities: m.capabilities.iter().map(|s| Ustr::from(s.as_str())).collect() }
}

fn placement_to(p: &ir::Placement) -> wire::Placement {
    wire::Placement { artifact: p.artifact.0, strategy: Some(placement_strategy_to(&p.strategy)) }
}
fn placement_from(m: &wire::Placement) -> ir::Placement {
    ir::Placement {
        artifact: ir::ArtifactId(m.artifact),
        strategy: m.strategy.as_ref().map(placement_strategy_from).unwrap_or(ir::PlacementStrategy::GithubArtifact),
    }
}
fn placement_strategy_to(s: &ir::PlacementStrategy) -> wire::PlacementStrategy {
    use wire::placement_strategy::Strategy;
    use ir::PlacementStrategy as P;
    let strategy = match s {
        P::GithubArtifact => Strategy::GithubArtifact(wire::Unit {}),
        P::SharedVolume { path } => Strategy::SharedVolume(wire::SharedVolume { path: path.clone() }),
        P::PersistentCache { key } => Strategy::PersistentCache(wire::PersistentCache { key: key.clone() }),
        P::LocalPath { path } => Strategy::LocalPath(wire::LocalPath { path: path.clone() }),
        P::OciRegistry { registry, tag } =>
            Strategy::OciRegistry(wire::OciRegistry { registry: registry.clone(), tag: tag.clone() }),
    };
    wire::PlacementStrategy { strategy: Some(strategy) }
}
fn placement_strategy_from(m: &wire::PlacementStrategy) -> ir::PlacementStrategy {
    use wire::placement_strategy::Strategy;
    use ir::PlacementStrategy as P;
    match &m.strategy {
        Some(Strategy::SharedVolume(v)) => P::SharedVolume { path: v.path.clone() },
        Some(Strategy::PersistentCache(v)) => P::PersistentCache { key: v.key.clone() },
        Some(Strategy::LocalPath(v)) => P::LocalPath { path: v.path.clone() },
        Some(Strategy::OciRegistry(v)) => P::OciRegistry { registry: v.registry.clone(), tag: v.tag.clone() },
        _ => P::GithubArtifact,
    }
}

fn action_to(a: &ir::Action) -> wire::Action {
    wire::Action {
        name: a.name.to_string(),
        op: a.op.to_string(),
        inputs: a.inputs.iter().map(|id| id.0).collect(),
        outputs: a.outputs.iter().map(|id| id.0).collect(),
        effects: a.effects.iter().map(|id| id.0).collect(),
        secrets: a.secrets.iter().map(|s| s.to_string()).collect(),
        actor_constraints: a.actor_constraints.iter().map(actor_constraint_to).collect(),
        shell: a.shell.as_ref().map(shell_to),
    }
}
fn action_from(m: &wire::Action) -> ir::Action {
    ir::Action {
        name: Ustr::from(m.name.as_str()),
        op: Ustr::from(m.op.as_str()),
        inputs: m.inputs.iter().map(|&i| ir::ArtifactId(i)).collect(),
        outputs: m.outputs.iter().map(|&i| ir::ArtifactId(i)).collect(),
        effects: m.effects.iter().map(|&i| ir::EffectId(i)).collect(),
        secrets: m.secrets.iter().map(|s| Ustr::from(s.as_str())).collect(),
        actor_constraints: m.actor_constraints.iter().map(actor_constraint_from).collect(),
        shell: m.shell.as_ref().map(shell_from),
    }
}

fn actor_constraint_to(c: &ir::ActorConstraint) -> wire::ActorConstraint {
    use wire::actor_constraint::Constraint;
    let constraint = match c {
        ir::ActorConstraint::Specific(id) => Constraint::Specific(id.0),
        ir::ActorConstraint::Label(l) => Constraint::Label(l.to_string()),
    };
    wire::ActorConstraint { constraint: Some(constraint) }
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
    wire::ShellAction { script: s.script.clone(), env: s.env.clone(), capture: capture_to(&s.capture) as i32 }
}
fn shell_from(m: &wire::ShellAction) -> ir::ShellAction {
    ir::ShellAction {
        script: m.script.clone(),
        env: m.env.clone(),
        capture: wire::CaptureRule::try_from(m.capture).map(capture_from).unwrap_or(ir::CaptureRule::NoCapture),
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
        let eff = b.effect("deploy", EffectKind::Deployment, true);
        b.add_effect_to(build, eff);
        b.effect("fx", EffectKind::Custom("weird-fx".into()), false);
        let actor = b.actor("big", &["self-hosted"], &["mount", "gpu"]);
        b.constrain_actor(checkout, actor);
        b.place(src, PlacementStrategy::SharedVolume { path: "/vol".into() });
        b.place(bin, PlacementStrategy::OciRegistry { registry: "ghcr.io".into(), tag: "v1".into() });
        b.max_parallel_jobs(4);
        b.build()
    }

    fn check(back: &Workflow) {
        assert_eq!(back.artifacts[2].ty, ArtifactType::Custom("thing".into()));
        assert_eq!(back.artifacts[1].producer, Some(ActionId(1)));
        assert_eq!(back.artifacts[1].path.as_deref(), Some("target/release/app"));
        assert_eq!(back.actions[1].secrets, vec!["TOKEN".to_string(), "KEY".to_string()]);
        assert!(matches!(back.actions[0].actor_constraints[0], ActorConstraint::Specific(ActorId(0))));
        assert_eq!(back.effects[0].kind, EffectKind::Deployment);
        assert!(back.effects[0].requires_approval);
        assert_eq!(back.effects[1].kind, EffectKind::Custom("weird-fx".into()));
        assert_eq!(back.actors[0].capabilities, vec!["mount".to_string(), "gpu".to_string()]);
        assert!(matches!(back.placements[1].strategy, PlacementStrategy::OciRegistry { .. }));
        assert_eq!(back.policies.max_parallel_jobs, Some(4));
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
        assert_eq!(Format::from_extension(Path::new("a.pb")), Some(Format::Binary));
        assert_eq!(Format::from_extension(Path::new("a.json")), Some(Format::Json));
        assert_eq!(Format::from_extension(Path::new("a.txt")), None);
    }
}

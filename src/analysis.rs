//! Static analysis facts derived from a workflow, shared by the planner and the
//! optimizer. These are the safety substrate for reordering passes: a pass must
//! never move work across an effect barrier.

use crate::ir::{ActionCallId, ArtifactId, ConsequenceKind, Workflow};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub struct Analysis {
    /// How many actions consume each artifact (informs materialize-once / mount).
    pub consumer_count: HashMap<ArtifactId, usize>,
    /// Actions with no declared effects — candidates for dedup/hoist/fusion.
    pub pure_actions: HashSet<ActionCallId>,
    /// Actions whose effects forbid reordering across them (deployments,
    /// releases, git writes, secret access, …). Reordering passes treat these
    /// as barriers.
    pub effect_barriers: HashSet<ActionCallId>,
}

impl Analysis {
    pub fn of(workflow: &Workflow) -> Analysis {
        let mut consumer_count: HashMap<ArtifactId, usize> = HashMap::new();
        for action in &workflow.action_calls {
            for &input in &action.inputs {
                *consumer_count.entry(input).or_default() += 1;
            }
        }

        let mut pure_actions = HashSet::new();
        let mut effect_barriers = HashSet::new();
        for (idx, action) in workflow.action_calls.iter().enumerate() {
            let id = ActionCallId(idx as u32);
            if action.consequences.is_empty() {
                pure_actions.insert(id);
            } else if action.consequences.iter().any(|&e| is_barrier(&workflow.consequence(e).kind)) {
                effect_barriers.insert(id);
            }
        }

        Analysis { consumer_count, pure_actions, effect_barriers }
    }

    pub fn consumers(&self, artifact: ArtifactId) -> usize {
        self.consumer_count.get(&artifact).copied().unwrap_or(0)
    }

    pub fn is_pure(&self, action: ActionCallId) -> bool {
        self.pure_actions.contains(&action)
    }

    pub fn is_barrier(&self, action: ActionCallId) -> bool {
        self.effect_barriers.contains(&action)
    }
}

/// Privileged, externally-observable effects that pin ordering.
fn is_barrier(kind: &ConsequenceKind) -> bool {
    matches!(kind,
        ConsequenceKind::Deployment | ConsequenceKind::PublishRelease
        | ConsequenceKind::GitWrite | ConsequenceKind::SecretAccess)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{ArtifactType, WorkflowBuilder};

    #[test]
    fn counts_consumers_and_classifies_effects() {
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::SourceTree);
        let bin = b.artifact("bin", ArtifactType::Binary);
        b.shell_action("checkout", "checkout", &[], &[src], "x");
        let build = b.shell_action("build", "build", &[src], &[bin], "make");
        let test = b.shell_action("test", "test", &[bin], &[], "t");
        let deploy = b.shell_action("deploy", "deploy", &[bin], &[], "d");
        let ship = b.consequence("ship", ConsequenceKind::Deployment, false);
        b.add_consequence_to(deploy, ship);
        let a = Analysis::of(&b.build());

        assert_eq!(a.consumers(src), 1);
        assert_eq!(a.consumers(bin), 2); // test + deploy
        assert!(a.is_pure(build));
        assert!(a.is_pure(test));
        assert!(a.is_barrier(deploy));
        assert!(!a.is_pure(deploy));
    }
}

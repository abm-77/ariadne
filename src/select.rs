//! Shared selection substrate.
//!
//! Both phases of the pipeline — implementation selection (plan-time, in
//! `lowering`) and instruction selection (emit-time, in `backends`) — are the
//! same pattern: *filter a set of capability-gated rules, rank them, pick the
//! best, explain why*. This module owns that pattern and the vocabulary it
//! speaks (`Capability`, `Stability`, `Candidate`), so the two layers are two
//! applications of one engine rather than two parallel copies.
//!
//! A rule's `requires` are HARD gates: a candidate is eligible only if every
//! required capability is available. Everything else (soft inventory
//! preference, instruction cost) is expressed as the caller's `priority` and
//! does not exclude — it only ranks.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ustr::Ustr;

/// An open capability string. The currency both layers gate on: inventory
/// implementations surface as `impl.<id>`, backend features as e.g.
/// `github.action_calls.uses`, actor abilities as `docker`, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capability(pub Ustr);

impl Capability {
    pub fn new(s: impl AsRef<str>) -> Self {
        Self(Ustr::from(s.as_ref()))
    }
}

/// Maturity of a rule. Used as a universal tiebreaker: prefer Stable, avoid
/// Deprecated/Experimental when something better is eligible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stability {
    Experimental,
    Beta,
    Stable,
    Deprecated,
}

/// Lower is better. The canonical ordering used by every selector.
pub fn stability_rank(s: Stability) -> u8 {
    match s {
        Stability::Stable => 0,
        Stability::Beta => 1,
        Stability::Experimental => 2,
        Stability::Deprecated => 3,
    }
}

/// A selectable rule. Implemented by `lowering::LoweringDef` and
/// `backends::Instruction`.
pub trait Candidate {
    /// The lookup key this rule is indexed under (a semantic action id for
    /// lowerings, a `LogicalOp` name for instructions). Selection gathers
    /// candidates by key before filtering and ranking.
    fn key(&self) -> &str;
    /// Hard capability gates: the rule is eligible only if all are available.
    fn requires(&self) -> &[Capability];
    fn stability(&self) -> Stability;
}

/// A keyed collection of selectable rules — the single container behind both
/// the lowering registry and the backend instruction catalogue. Holds the rules
/// and a by-key index; selection is `candidates(key)` (this module's job) then
/// `resolve` (filter + rank). Domain-specific methods (`builtin`, `select`)
/// attach to the concrete specializations in `lowering` and `backends`.
#[derive(Clone)]
pub struct Registry<C> {
    items: Vec<C>,
    by_key: HashMap<String, Vec<usize>>,
}

impl<C: Candidate> Registry<C> {
    pub fn new() -> Self {
        Self { items: Vec::new(), by_key: HashMap::new() }
    }

    pub fn from_items(items: Vec<C>) -> Self {
        let mut r = Self::new();
        r.register_all(items);
        r
    }

    pub fn register(&mut self, c: C) {
        let i = self.items.len();
        self.by_key.entry(c.key().to_string()).or_default().push(i);
        self.items.push(c);
    }

    pub fn register_all(&mut self, items: impl IntoIterator<Item = C>) {
        for c in items {
            self.register(c);
        }
    }

    pub fn all(&self) -> &[C] {
        &self.items
    }

    /// Rules registered under `key`, in registration order.
    pub fn candidates<'a>(&'a self, key: &str) -> impl Iterator<Item = &'a C> {
        self.by_key.get(key).into_iter().flatten().map(move |&i| &self.items[i])
    }
}

impl<C: Candidate> Default for Registry<C> {
    fn default() -> Self {
        Self::new()
    }
}

/// True if every capability the candidate requires is in `available`.
pub fn requirements_met(c: &impl Candidate, available: &[Capability]) -> bool {
    c.requires().iter().all(|r| available.contains(r))
}

/// Resolve the best candidate: among those whose hard requirements are
/// satisfied by `available`, minimize `(priority(c), stability)`. Ties break by
/// iteration order — `min_by_key` is stable — so registration order is the
/// final, deterministic tiebreaker. `priority` injects the caller's domain
/// ranking (lowering: inventory preference class; instruction: cost); lower
/// wins. Returns `None` when nothing is eligible.
pub fn resolve<'a, C: Candidate + 'a>(
    candidates: impl IntoIterator<Item = &'a C>,
    available: &[Capability],
    priority: impl Fn(&C) -> u32,
) -> Option<&'a C> {
    candidates
        .into_iter()
        .filter(|c| requirements_met(*c, available))
        .min_by_key(|c| (priority(c), stability_rank(c.stability())))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rule {
        key: &'static str,
        requires: Vec<Capability>,
        stability: Stability,
        cost: u32,
    }
    impl Candidate for Rule {
        fn key(&self) -> &str { self.key }
        fn requires(&self) -> &[Capability] { &self.requires }
        fn stability(&self) -> Stability { self.stability }
    }

    fn rule(reqs: &[&str], stability: Stability, cost: u32) -> Rule {
        Rule { key: "k", requires: reqs.iter().map(Capability::new).collect(), stability, cost }
    }

    #[test]
    fn excludes_candidates_with_unmet_requirements() {
        let rules = [rule(&["docker"], Stability::Stable, 1)];
        assert!(resolve(rules.iter(), &[], |r| r.cost).is_none());
        let avail = vec![Capability::new("docker")];
        assert!(resolve(rules.iter(), &avail, |r| r.cost).is_some());
    }

    #[test]
    fn ranks_by_priority_then_stability_then_order() {
        let rules = [
            rule(&[], Stability::Stable, 5),   // 0: higher cost
            rule(&[], Stability::Stable, 1),   // 1: cheapest -> wins
            rule(&[], Stability::Stable, 1),   // 2: same cost, later -> loses to 1
        ];
        let picked = resolve(rules.iter(), &[], |r| r.cost).unwrap();
        assert_eq!(picked.cost, 1);
        assert!(std::ptr::eq(picked, &rules[1]));
    }

    #[test]
    fn stability_breaks_priority_ties() {
        let rules = [rule(&[], Stability::Experimental, 1),
            rule(&[], Stability::Stable, 1)];
        let picked = resolve(rules.iter(), &[], |r| r.cost).unwrap();
        assert_eq!(picked.stability, Stability::Stable);
    }

    #[test]
    fn registry_indexes_candidates_by_key() {
        let mut reg = Registry::new();
        reg.register(Rule { key: "a", requires: vec![], stability: Stability::Stable, cost: 1 });
        reg.register(Rule { key: "b", requires: vec![], stability: Stability::Stable, cost: 1 });
        reg.register(Rule { key: "a", requires: vec![], stability: Stability::Stable, cost: 2 });
        assert_eq!(reg.candidates("a").count(), 2);
        assert_eq!(reg.candidates("b").count(), 1);
        assert_eq!(reg.candidates("missing").count(), 0);
        assert_eq!(reg.all().len(), 3);
    }
}

//! Cost model for the optimizer. A plan's cost is compared along the workflow's
//! *user-defined* ordered list of objectives (`Policies.objectives`); the first
//! objective that differs decides, the rest break ties.

use crate::ir::Objective;
use crate::planner::{AccessMode, LogicalOp, Plan};
use crate::profile::Profile;
use std::cmp::Ordering;
use std::collections::HashMap;
use ustr::Ustr;

/// Assumed action duration (seconds) when the profile has no measurement.
const DEFAULT_ACTION_SECS: f64 = 30.0;
/// Assumed transfer bandwidth (bytes/second) for copied artifacts.
const TRANSFER_BYTES_PER_SEC: f64 = 100_000_000.0;
/// Assumed fixed per-job overhead (seconds): runner provisioning + workspace
/// setup, paid once per execution unit. This is what packing independent units
/// into one job avoids; profile `setup_times` overrides it per runner.
const DEFAULT_JOB_OVERHEAD_SECS: f64 = 10.0;

/// An estimated cost along all tracked dimensions. `seconds` is the wall-clock
/// critical path; `dollars` is summed machine-time across all units.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Cost {
    pub seconds: f64,
    pub transfer_bytes: u64,
    pub dollars: f64,
}

impl Cost {
    // Deliberately a plain method, not `std::ops::Add`: costs are summed
    // explicitly during estimation, never via the `+` operator.
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, o: Cost) -> Cost {
        Cost {
            seconds: self.seconds + o.seconds,
            transfer_bytes: self.transfer_bytes + o.transfer_bytes,
            dollars: self.dollars + o.dollars,
        }
    }

    /// Estimate a plan's cost from profile data. `seconds` is the schedule
    /// makespan: a list-schedule of the `needs` DAG onto `plan.max_parallel_jobs`
    /// concurrency slots (unbounded when the policy is silent), where each unit's
    /// duration is its fixed setup overhead + compute + queue + copy-transfer
    /// time. `transfer_bytes` and `dollars` are summed across all units. With no
    /// profile, durations fall back to constants so plans stay comparable.
    ///
    /// Modeling the per-job overhead and the concurrency bound is what lets the
    /// optimizer reason about packing: under a cap, independent units serialize
    /// regardless, so merging them removes redundant setup/transfer for free;
    /// with runners to spare, merging only lengthens the makespan.
    pub fn estimate(plan: &Plan, profile: &Profile) -> Cost {
        let mut durs: HashMap<Ustr, f64> = HashMap::new();
        let mut total_bytes = 0u64;
        let mut total_dollars = 0.0;
        for u in &plan.units {
            let bytes = unit_transfer_bytes(u, profile);
            let setup = profile.setup_times.get(u.runner.as_str()).copied().unwrap_or(DEFAULT_JOB_OVERHEAD_SECS);
            let compute = unit_compute_secs(u, profile);
            let queue = profile.queue_times.get(u.runner.as_str()).copied().unwrap_or(0.0);
            let dur = setup + compute + queue + bytes as f64 / TRANSFER_BYTES_PER_SEC;

            let rate = profile.runner_costs.get(u.runner.as_str()).copied().unwrap_or(0.0);
            total_bytes += bytes;
            total_dollars += dur * rate;
            durs.insert(u.id, dur);
        }
        Cost { seconds: makespan(plan, &durs), transfer_bytes: total_bytes, dollars: total_dollars }
    }

    /// Compare two costs by the given objective priority order (lexicographic).
    /// Floats use `total_cmp` for determinism.
    pub fn cmp_by(&self, other: &Cost, objectives: &[Objective]) -> Ordering {
        for obj in objectives {
            let ord = match obj {
                Objective::CriticalPath => self.seconds.total_cmp(&other.seconds),
                Objective::TransferBytes => self.transfer_bytes.cmp(&other.transfer_bytes),
                Objective::DollarCost => self.dollars.total_cmp(&other.dollars),
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    }
}

/// List-schedule the units onto `plan.max_parallel_jobs` concurrency slots and
/// return the makespan. Units are processed in topological order (the planner
/// emits them so; `needs` always point to earlier units). Each ready unit takes
/// a fresh slot if one is free, else the earliest-freeing slot; with no cap,
/// every unit gets its own slot and the makespan equals the critical path.
fn makespan(plan: &Plan, durs: &HashMap<Ustr, f64>) -> f64 {
    let m = plan.max_parallel_jobs.filter(|&n| n > 0).unwrap_or(usize::MAX);
    let mut finish: HashMap<Ustr, f64> = HashMap::new();
    let mut slots: Vec<f64> = Vec::new(); // free-at time of each allocated slot
    for u in &plan.units {
        let ready = u.needs.iter().filter_map(|n| finish.get(n)).copied().fold(0.0, f64::max);
        let dur = durs.get(&u.id).copied().unwrap_or(0.0);
        let start = if slots.len() < m {
            slots.push(0.0); // a slot we can occupy from `ready`
            ready
        } else {
            // Reuse the earliest-freeing slot (lowest index breaks ties).
            let idx = slots.iter().enumerate()
                .min_by(|(_, a), (_, b)| a.total_cmp(b)).map(|(i, _)| i).unwrap();
            let start = ready.max(slots[idx]);
            slots[idx] = start + dur;
            finish.insert(u.id, start + dur);
            continue;
        };
        *slots.last_mut().unwrap() = start + dur;
        finish.insert(u.id, start + dur);
    }
    finish.into_values().fold(0.0, f64::max)
}

/// A unit's compute time: the sum of its shell ops' durations (each `RunShell`
/// is labelled with its action name, so a fused unit holding several actions
/// costs the sum of their work, not a single lookup). Units with no shell op
/// (e.g. a native checkout) fall back to the action-name duration. Missing
/// measurements use a constant so plans stay comparable.
fn unit_compute_secs(unit: &crate::planner::ExecutionUnit, profile: &Profile) -> f64 {
    let mut shell_secs = 0.0;
    let mut shell_ops = 0u32;
    for op in &unit.ops {
        if let LogicalOp::RunShell { label, .. } = op {
            shell_secs += profile.action_duration(label.as_str()).unwrap_or(DEFAULT_ACTION_SECS);
            shell_ops += 1;
        }
    }
    if shell_ops > 0 {
        shell_secs
    } else {
        profile.action_duration(unit.action_name.as_str()).unwrap_or(DEFAULT_ACTION_SECS)
    }
}

/// Bytes a unit moves by copy: downloaded artifacts and copy-mode transfers
/// (mounts, same-host paths and OCI layers move no bytes between units).
fn unit_transfer_bytes(unit: &crate::planner::ExecutionUnit, profile: &Profile) -> u64 {
    unit.ops.iter().filter_map(|op| match op {
        LogicalOp::DownloadArtifact { name, .. } => profile.artifact_size(name.as_str()),
        LogicalOp::TransferArtifact { name, access: AccessMode::Copy, .. } => profile.artifact_size(name.as_str()),
        _ => None,
    }).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ir::default_objectives;

    #[test]
    fn objective_order_decides_ties() {
        let a = Cost { seconds: 10.0, transfer_bytes: 100, dollars: 1.0 };
        let b = Cost { seconds: 10.0, transfer_bytes: 50, dollars: 5.0 };
        // Default order: equal time -> fewer bytes wins (b < a).
        assert_eq!(a.cmp_by(&b, &default_objectives()), Ordering::Greater);
        // Dollars-first: a is cheaper -> a < b.
        let dollars_first = [Objective::DollarCost, Objective::CriticalPath];
        assert_eq!(a.cmp_by(&b, &dollars_first), Ordering::Less);
    }

    #[test]
    fn critical_path_dominates() {
        let a = Cost { seconds: 5.0, transfer_bytes: 999, dollars: 999.0 };
        let b = Cost { seconds: 9.0, transfer_bytes: 0, dollars: 0.0 };
        assert_eq!(a.cmp_by(&b, &default_objectives()), Ordering::Less);
    }

    #[test]
    fn estimate_uses_critical_path_and_profile() {
        use crate::ir::{ArtifactType, WorkflowBuilder};
        let mut b = WorkflowBuilder::new("w");
        let src = b.artifact("src", ArtifactType::Binary);
        let bin = b.artifact("bin", ArtifactType::Binary);
        b.shell_action("checkout", "checkout", &[], &[src], "co");
        b.shell_action("build", "build", &[src], &[bin], "make");
        b.actor("r", &["ubuntu-latest"], &[]);
        let plan = crate::planner::plan(&b.build()).unwrap();

        let mut p = Profile::default();
        p.action_durations.insert("checkout".into(), 10.0);
        p.action_durations.insert("build".into(), 20.0);
        p.runner_costs.insert("ubuntu-latest".into(), 0.01);
        p.artifact_sizes.insert("src".into(), 100_000_000); // 1s to copy at 100MB/s

        let cost = Cost::estimate(&plan, &p);
        // Each unit pays the default 10s setup. checkout = 10+10 = 20; build =
        // 10 + src copy(1s) + 20 = 31. Unbounded slots → critical path 20+31 = 51.
        assert!((cost.seconds - 51.0).abs() < 1e-6, "seconds = {}", cost.seconds);
        assert_eq!(cost.transfer_bytes, 100_000_000);
        assert!(cost.dollars > 0.0);
    }

    #[test]
    fn makespan_respects_concurrency_cap() {
        use crate::ir::{ArtifactType, WorkflowBuilder};
        // Three independent jobs, each 40s (10 setup + 30 default compute).
        let mut b = WorkflowBuilder::new("w");
        for i in 0..3 {
            let out = b.artifact(&format!("a{i}"), ArtifactType::Binary);
            b.shell_action(&format!("job{i}"), "build", &[], &[out], "make");
        }
        b.actor("r", &["ubuntu-latest"], &[]);
        b.max_parallel_jobs(1);
        let plan = crate::planner::plan(&b.build()).unwrap();
        // One slot → all three serialize: 3 * 40 = 120.
        let capped = Cost::estimate(&plan, &Profile::default());
        assert!((capped.seconds - 120.0).abs() < 1e-6, "capped = {}", capped.seconds);
    }

    #[test]
    fn unbounded_makespan_is_critical_path() {
        use crate::ir::{ArtifactType, WorkflowBuilder};
        let mut b = WorkflowBuilder::new("w");
        for i in 0..3 {
            let out = b.artifact(&format!("a{i}"), ArtifactType::Binary);
            b.shell_action(&format!("job{i}"), "build", &[], &[out], "make");
        }
        b.actor("r", &["ubuntu-latest"], &[]);
        let plan = crate::planner::plan(&b.build()).unwrap();
        // No cap → all three run concurrently: makespan = one job = 40.
        let cost = Cost::estimate(&plan, &Profile::default());
        assert!((cost.seconds - 40.0).abs() < 1e-6, "seconds = {}", cost.seconds);
    }

    #[test]
    fn empty_profile_still_estimates() {
        use crate::ir::{ArtifactType, WorkflowBuilder};
        let mut b = WorkflowBuilder::new("w");
        let bin = b.artifact("bin", ArtifactType::Binary);
        b.shell_action("build", "build", &[], &[bin], "make");
        b.actor("r", &["ubuntu-latest"], &[]);
        let plan = crate::planner::plan(&b.build()).unwrap();
        let cost = Cost::estimate(&plan, &Profile::default());
        assert_eq!(cost.seconds, DEFAULT_ACTION_SECS + DEFAULT_JOB_OVERHEAD_SECS);
        assert_eq!(cost.transfer_bytes, 0);
    }
}

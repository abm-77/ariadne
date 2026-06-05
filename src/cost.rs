//! Cost model for the optimizer. A plan's cost is compared along the workflow's
//! *user-defined* ordered list of objectives (`Policies.objectives`); the first
//! objective that differs decides, the rest break ties.

use crate::ir::Objective;
use crate::planner::{AccessMode, PhysicalOp, Plan};
use crate::profile::Profile;
use std::cmp::Ordering;
use std::collections::HashMap;
use ustr::Ustr;

/// Assumed action duration (seconds) when the profile has no measurement.
const DEFAULT_ACTION_SECS: f64 = 30.0;
/// Assumed transfer bandwidth (bytes/second) for copied artifacts.
const TRANSFER_BYTES_PER_SEC: f64 = 100_000_000.0;

/// An estimated cost along all tracked dimensions. `seconds` is the wall-clock
/// critical path; `dollars` is summed machine-time across all units.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Cost {
    pub seconds: f64,
    pub transfer_bytes: u64,
    pub dollars: f64,
}

impl Cost {
    pub fn add(self, o: Cost) -> Cost {
        Cost {
            seconds: self.seconds + o.seconds,
            transfer_bytes: self.transfer_bytes + o.transfer_bytes,
            dollars: self.dollars + o.dollars,
        }
    }

    /// Estimate a plan's cost from profile data. `seconds` is the critical path
    /// through the `needs` DAG (each unit's compute + queue + copy-transfer
    /// time); `transfer_bytes` and `dollars` are summed across all units. With
    /// no profile, durations fall back to a constant so plans stay comparable.
    pub fn estimate(plan: &Plan, profile: &Profile) -> Cost {
        let mut finish: HashMap<Ustr, f64> = HashMap::new();
        let mut total_bytes = 0u64;
        let mut total_dollars = 0.0;
        let mut critical = 0.0_f64;
        for u in &plan.units {
            let bytes = unit_transfer_bytes(u, profile);
            let compute = profile.action_duration(u.action_name.as_str()).unwrap_or(DEFAULT_ACTION_SECS);
            let queue = profile.queue_times.get(u.runner.as_str()).copied().unwrap_or(0.0);
            let dur = compute + queue + bytes as f64 / TRANSFER_BYTES_PER_SEC;

            let rate = profile.runner_costs.get(u.runner.as_str()).copied().unwrap_or(0.0);
            total_bytes += bytes;
            total_dollars += dur * rate;

            let start = u.needs.iter().filter_map(|n| finish.get(n)).copied().fold(0.0, f64::max);
            let f = start + dur;
            finish.insert(u.id, f);
            critical = critical.max(f);
        }
        Cost { seconds: critical, transfer_bytes: total_bytes, dollars: total_dollars }
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

/// Bytes a unit moves by copy: downloaded artifacts and copy-mode transfers
/// (mounts, same-host paths and OCI layers move no bytes between units).
fn unit_transfer_bytes(unit: &crate::planner::ExecutionUnit, profile: &Profile) -> u64 {
    unit.ops.iter().filter_map(|op| match op {
        PhysicalOp::DownloadArtifact { name, .. } => profile.artifact_size(name.as_str()),
        PhysicalOp::TransferArtifact { name, access: AccessMode::Copy, .. } => profile.artifact_size(name.as_str()),
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
        let src = b.artifact("src", ArtifactType::SourceTree);
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
        // Critical path: checkout(10) -> build downloads src(1s) + build(20) = 31.
        assert!((cost.seconds - 31.0).abs() < 1e-6, "seconds = {}", cost.seconds);
        assert_eq!(cost.transfer_bytes, 100_000_000);
        assert!(cost.dollars > 0.0);
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
        assert_eq!(cost.seconds, DEFAULT_ACTION_SECS);
        assert_eq!(cost.transfer_bytes, 0);
    }
}

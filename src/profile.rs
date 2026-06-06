//! Profile data from previous runs. Feeds the optimizer's cost model; it may
//! improve plans but MUST NEVER change workflow semantics.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    /// Observed artifact sizes in bytes, by artifact name.
    #[serde(default)]
    pub artifact_sizes: HashMap<String, u64>,
    /// Observed action durations in seconds, by action name.
    #[serde(default)]
    pub action_durations: HashMap<String, f64>,
    /// Cache hit rate (0..1), by cache key.
    #[serde(default)]
    pub cache_hit_rates: HashMap<String, f64>,
    /// Queue/scheduling delay in seconds, by runner label.
    #[serde(default)]
    pub queue_times: HashMap<String, f64>,
    /// Fixed per-job setup cost in seconds (runner provisioning, workspace
    /// preparation), by runner label. Paid once per execution unit, so packing
    /// independent units into one job pays it once instead of per unit.
    #[serde(default)]
    pub setup_times: HashMap<String, f64>,
    /// Runner price in dollars per second, by runner label.
    #[serde(default)]
    pub runner_costs: HashMap<String, f64>,
    /// Actor utilization (0..1), by actor name. Drives actor optimization:
    /// under-utilized → seek a smaller/cheaper fit; over-utilized → seek an
    /// alternative to reduce contention.
    #[serde(default)]
    pub actor_utilization: HashMap<String, f64>,
    /// Failure rate (0..1), by action name.
    #[serde(default)]
    pub failure_rates: HashMap<String, f64>,
}

impl Profile {
    pub fn load(path: &Path) -> Result<Profile, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read profile '{}': {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("parse profile '{}': {e}", path.display()))
    }

    pub fn artifact_size(&self, name: &str) -> Option<u64> {
        self.artifact_sizes.get(name).copied()
    }

    pub fn action_duration(&self, name: &str) -> Option<f64> {
        self.action_durations.get(name).copied()
    }

    pub fn utilization(&self, actor: &str) -> Option<f64> {
        self.actor_utilization.get(actor).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_profile_has_no_data() {
        let p = Profile::default();
        assert_eq!(p.artifact_size("x"), None);
    }

    #[test]
    fn parses_from_json() {
        let p: Profile = serde_json::from_str(
            r#"{ "artifact_sizes": {"model": 21474836480}, "actor_utilization": {"gpu": 0.95} }"#,
        )
        .unwrap();
        assert_eq!(p.artifact_size("model"), Some(21474836480));
        assert_eq!(p.utilization("gpu"), Some(0.95));
    }
}

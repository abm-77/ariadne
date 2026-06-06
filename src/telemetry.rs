//! Profile collection: turning real CI run telemetry back into a `Profile` that
//! feeds the optimizer's cost model. This is the inverse of emission and closes
//! the profile-guided loop (run -> measure -> plan better).
//!
//! The model is backend-extensible, mirroring emission. Each backend implements
//! a `ProfileCollector` that parses *its* platform's run records (GitHub REST
//! JSON, local executor logs, ...) into the backend-agnostic `RunReport`. The
//! aggregation of many reports into a `Profile` is shared, so a new backend only
//! has to map its native telemetry onto `RunReport`.

use crate::profile::Profile;
use std::collections::{BTreeMap, HashMap, HashSet};

/// One backend's raw telemetry for a single run, an opaque payload the owning
/// collector knows how to parse (e.g. GitHub bundles a run's jobs + artifacts
/// JSON). The CLI obtains these; the collector interprets them.
pub struct RawRun(pub String);

/// A normalized, backend-agnostic record of one executed run. Backends map their
/// native telemetry onto this; `aggregate` turns a batch into a `Profile`.
#[derive(Debug, Clone, Default)]
pub struct RunReport {
    pub units: Vec<UnitReport>,
    pub artifacts: Vec<ArtifactReport>,
}

/// One execution unit (a job) in a run.
#[derive(Debug, Clone)]
pub struct UnitReport {
    /// The runner label the unit ran on (matches the plan's `runner`).
    pub runner_label: String,
    /// Scheduling delay before the unit started (started - created), seconds.
    pub queue_secs: Option<f64>,
    /// Fixed per-job setup overhead (runner provisioning, workspace prep), s.
    pub setup_secs: Option<f64>,
    /// The semantic actions that ran in this unit.
    pub actions: Vec<ActionReport>,
}

/// One semantic action's compute within a unit.
#[derive(Debug, Clone)]
pub struct ActionReport {
    pub action: String,
    pub duration_secs: Option<f64>,
    pub failed: bool,
}

/// One produced artifact and its observed size.
#[derive(Debug, Clone)]
pub struct ArtifactReport {
    pub name: String,
    pub size_bytes: u64,
}

/// Backend-owned parser from native run telemetry to normalized reports, plus
/// the static per-runner pricing the backend knows. Object-safe so the CLI can
/// dispatch by backend id.
pub trait ProfileCollector: Send + Sync {
    fn id(&self) -> &str;
    /// Parse a batch of raw per-run telemetry into normalized reports.
    fn parse(&self, runs: &[RawRun]) -> Result<Vec<RunReport>, String>;
    /// Known runner price in dollars per second, by runner label. Empty by
    /// default (e.g. self-hosted has no list price).
    fn runner_pricing(&self) -> HashMap<String, f64> { HashMap::new() }
}

/// Aggregate many run reports into a `Profile` by averaging each observed
/// quantity, then attach known runner pricing for the labels actually seen.
/// Durations/sizes are means; failure rate is failures over executions.
pub fn aggregate(runs: &[RunReport], pricing: &HashMap<String, f64>) -> Profile {
    let mut dur: HashMap<String, Mean> = HashMap::new();
    let mut setup: HashMap<String, Mean> = HashMap::new();
    let mut queue: HashMap<String, Mean> = HashMap::new();
    let mut size: HashMap<String, Mean> = HashMap::new();
    let mut fails: HashMap<String, (u32, u32)> = HashMap::new(); // (failures, executions)
    let mut labels: HashSet<String> = HashSet::new();

    for run in runs {
        for u in &run.units {
            labels.insert(u.runner_label.clone());
            if let Some(q) = u.queue_secs { queue.entry(u.runner_label.clone()).or_default().add(q); }
            if let Some(s) = u.setup_secs { setup.entry(u.runner_label.clone()).or_default().add(s); }
            for a in &u.actions {
                if let Some(d) = a.duration_secs { dur.entry(a.action.clone()).or_default().add(d); }
                let e = fails.entry(a.action.clone()).or_default();
                e.1 += 1;
                if a.failed { e.0 += 1; }
            }
        }
        for art in &run.artifacts {
            size.entry(art.name.clone()).or_default().add(art.size_bytes as f64);
        }
    }

    Profile {
        action_durations: means(&dur),
        setup_times: means(&setup),
        queue_times: means(&queue),
        artifact_sizes: size.iter().map(|(k, m)| (k.clone(), m.mean().round() as u64)).collect(),
        failure_rates: fails.iter()
            .filter(|(_, (_, n))| *n > 0)
            .map(|(k, (f, n))| (k.clone(), *f as f64 / *n as f64))
            .collect(),
        runner_costs: labels.iter()
            .filter_map(|l| pricing.get(l).map(|c| (l.clone(), *c)))
            .collect(),
        ..Profile::default()
    }
}

#[derive(Default, Clone, Copy)]
struct Mean { sum: f64, n: u32 }
impl Mean {
    fn add(&mut self, x: f64) { self.sum += x; self.n += 1; }
    fn mean(&self) -> f64 { if self.n == 0 { 0.0 } else { self.sum / self.n as f64 } }
}

fn means(m: &HashMap<String, Mean>) -> HashMap<String, f64> {
    m.iter().map(|(k, v)| (k.clone(), v.mean())).collect()
}

/// Registry of profile collectors by backend id, mirroring `BackendRegistry`.
pub struct CollectorRegistry {
    collectors: BTreeMap<String, Box<dyn ProfileCollector>>,
}

impl CollectorRegistry {
    pub fn with_builtins() -> Self {
        let mut r = CollectorRegistry { collectors: BTreeMap::new() };
        r.insert(Box::new(crate::backends::github::GithubCollector));
        r
    }

    fn insert(&mut self, c: Box<dyn ProfileCollector>) {
        self.collectors.insert(c.id().to_string(), c);
    }

    pub fn ids(&self) -> Vec<&str> {
        self.collectors.keys().map(|s| s.as_str()).collect()
    }

    pub fn resolve(&self, id: &str) -> Option<&dyn ProfileCollector> {
        self.collectors.get(id).map(|b| b.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(label: &str, action: &str, dur: f64, setup: f64, queue: f64) -> RunReport {
        RunReport {
            units: vec![UnitReport {
                runner_label: label.into(),
                queue_secs: Some(queue),
                setup_secs: Some(setup),
                actions: vec![ActionReport { action: action.into(), duration_secs: Some(dur), failed: false }],
            }],
            artifacts: vec![ArtifactReport { name: format!("{action}/out"), size_bytes: 1000 }],
        }
    }

    #[test]
    fn averages_across_runs() {
        let runs = vec![
            report("ubuntu-latest", "build", 10.0, 4.0, 1.0),
            report("ubuntu-latest", "build", 20.0, 6.0, 3.0),
        ];
        let pricing = HashMap::from([("ubuntu-latest".to_string(), 0.0001)]);
        let p = aggregate(&runs, &pricing);
        assert_eq!(p.action_duration("build"), Some(15.0));
        assert_eq!(p.setup_times.get("ubuntu-latest").copied(), Some(5.0));
        assert_eq!(p.queue_times.get("ubuntu-latest").copied(), Some(2.0));
        assert_eq!(p.artifact_size("build/out"), Some(1000));
        assert_eq!(p.runner_costs.get("ubuntu-latest").copied(), Some(0.0001));
    }

    #[test]
    fn failure_rate_is_failures_over_executions() {
        let ok = ActionReport { action: "test".into(), duration_secs: Some(5.0), failed: false };
        let bad = ActionReport { action: "test".into(), duration_secs: Some(5.0), failed: true };
        let mk = |a: ActionReport| RunReport {
            units: vec![UnitReport { runner_label: "r".into(), queue_secs: None, setup_secs: None, actions: vec![a] }],
            artifacts: vec![],
        };
        let runs = vec![mk(ok.clone()), mk(ok), mk(bad.clone()), mk(bad)];
        let p = aggregate(&runs, &HashMap::new());
        assert_eq!(p.failure_rates.get("test").copied(), Some(0.5));
    }

    #[test]
    fn pricing_only_for_observed_labels() {
        let runs = vec![report("ubuntu-latest", "build", 10.0, 4.0, 1.0)];
        let pricing = HashMap::from([
            ("ubuntu-latest".to_string(), 0.0001),
            ("macos-latest".to_string(), 0.001),
        ]);
        let p = aggregate(&runs, &pricing);
        assert!(p.runner_costs.contains_key("ubuntu-latest"));
        assert!(!p.runner_costs.contains_key("macos-latest"), "unobserved label not priced");
    }
}

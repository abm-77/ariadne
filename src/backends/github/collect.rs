//! GitHub Actions profile collector: parses run telemetry from the REST API
//! (a run's jobs + artifacts responses) into normalized `RunReport`s. This is
//! the inverse of `github::emit`, so it relies on the same naming contract:
//! a job's id is the unit id, a step named `Run <action>` is that action's
//! compute, the implicit `Set up job` step is the per-job setup overhead, and
//! an artifact is named `<action>/<output>`.

use crate::telemetry::{
    ActionReport, ArtifactReport, ProfileCollector, RawRun, RunReport, UnitReport,
};
use chrono::DateTime;
use serde::Deserialize;
use std::collections::HashMap;

pub struct GithubCollector;

/// One run's telemetry: the `actions/runs/{id}/jobs` and `.../artifacts`
/// responses bundled together (as fetched by `loom profile`).
#[derive(Deserialize, Default)]
struct Bundle {
    #[serde(default)]
    jobs: JobsResponse,
    #[serde(default)]
    artifacts: ArtifactsResponse,
}

#[derive(Deserialize, Default)]
struct JobsResponse {
    #[serde(default)]
    jobs: Vec<Job>,
}

#[derive(Deserialize)]
struct Job {
    #[serde(default)]
    labels: Vec<String>,
    created_at: Option<String>,
    started_at: Option<String>,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Deserialize)]
struct Step {
    #[serde(default)]
    name: String,
    started_at: Option<String>,
    completed_at: Option<String>,
    conclusion: Option<String>,
}

#[derive(Deserialize, Default)]
struct ArtifactsResponse {
    #[serde(default)]
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
struct Artifact {
    #[serde(default)]
    name: String,
    #[serde(default)]
    size_in_bytes: u64,
}

/// The GitHub step name that records runner provisioning / workspace setup.
const SETUP_STEP: &str = "Set up job";

impl ProfileCollector for GithubCollector {
    fn id(&self) -> &str {
        "github"
    }

    fn parse(&self, runs: &[RawRun]) -> Result<Vec<RunReport>, String> {
        runs.iter().map(|r| parse_run(&r.0)).collect()
    }

    fn runner_pricing(&self) -> HashMap<String, f64> {
        pricing()
    }
}

fn parse_run(json: &str) -> Result<RunReport, String> {
    let bundle: Bundle =
        serde_json::from_str(json).map_err(|e| format!("invalid GitHub run telemetry: {e}"))?;

    let units = bundle
        .jobs
        .jobs
        .iter()
        .map(|job| {
            let actions = job
                .steps
                .iter()
                .filter_map(|s| {
                    s.name.strip_prefix("Run ").map(|action| ActionReport {
                        action: action.to_string(),
                        duration_secs: secs_between(&s.started_at, &s.completed_at),
                        failed: matches!(s.conclusion.as_deref(), Some("failure")),
                    })
                })
                .collect();
            let setup_secs = job
                .steps
                .iter()
                .find(|s| s.name == SETUP_STEP)
                .and_then(|s| secs_between(&s.started_at, &s.completed_at));
            UnitReport {
                runner_label: job.labels.first().cloned().unwrap_or_default(),
                queue_secs: secs_between(&job.created_at, &job.started_at),
                setup_secs,
                actions,
            }
        })
        .collect();

    let artifacts = bundle
        .artifacts
        .artifacts
        .iter()
        .map(|a| ArtifactReport {
            name: a.name.clone(),
            size_bytes: a.size_in_bytes,
        })
        .collect();

    Ok(RunReport { units, artifacts })
}

/// Seconds between two RFC3339 timestamps (`b - a`); `None` if either is missing
/// or unparseable.
fn secs_between(a: &Option<String>, b: &Option<String>) -> Option<f64> {
    let a = DateTime::parse_from_rfc3339(a.as_deref()?).ok()?;
    let b = DateTime::parse_from_rfc3339(b.as_deref()?).ok()?;
    Some((b - a).num_milliseconds() as f64 / 1000.0)
}

/// GitHub-hosted runner list prices in dollars per second (per-minute price / 60,
/// as published for private repositories; Linux/Windows/macOS standard runners).
fn pricing() -> HashMap<String, f64> {
    let per_min: &[(&str, f64)] = &[
        ("ubuntu-latest", 0.008),
        ("ubuntu-24.04", 0.008),
        ("ubuntu-22.04", 0.008),
        ("ubuntu-20.04", 0.008),
        ("windows-latest", 0.016),
        ("windows-2022", 0.016),
        ("windows-2019", 0.016),
        ("macos-latest", 0.08),
        ("macos-14", 0.08),
        ("macos-13", 0.08),
    ];
    per_min
        .iter()
        .map(|(label, m)| (label.to_string(), m / 60.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> &'static str {
        r#"{
          "jobs": { "jobs": [
            {
              "labels": ["ubuntu-latest"],
              "created_at": "2024-01-01T00:00:00Z",
              "started_at": "2024-01-01T00:00:05Z",
              "steps": [
                {"name": "Set up job", "started_at": "2024-01-01T00:00:05Z", "completed_at": "2024-01-01T00:00:09Z", "conclusion": "success"},
                {"name": "Run build_loom", "started_at": "2024-01-01T00:00:09Z", "completed_at": "2024-01-01T00:00:39Z", "conclusion": "success"},
                {"name": "Run test_workspace", "started_at": "2024-01-01T00:00:39Z", "completed_at": "2024-01-01T00:01:39Z", "conclusion": "failure"},
                {"name": "Upload artifact build_loom/loom", "started_at": "2024-01-01T00:01:39Z", "completed_at": "2024-01-01T00:01:42Z", "conclusion": "success"}
              ]
            }
          ] },
          "artifacts": { "artifacts": [
            {"name": "build_loom/loom", "size_in_bytes": 5242880}
          ] }
        }"#
    }

    #[test]
    fn maps_steps_and_setup_and_queue() {
        let run = parse_run(bundle()).unwrap();
        let unit = &run.units[0];
        assert_eq!(unit.runner_label, "ubuntu-latest");
        assert_eq!(unit.queue_secs, Some(5.0));
        assert_eq!(unit.setup_secs, Some(4.0));
        // Only "Run X" steps become actions (not Set up job / Upload artifact).
        assert_eq!(unit.actions.len(), 2);
        let build = unit
            .actions
            .iter()
            .find(|a| a.action == "build_loom")
            .unwrap();
        assert_eq!(build.duration_secs, Some(30.0));
        assert!(!build.failed);
        let test = unit
            .actions
            .iter()
            .find(|a| a.action == "test_workspace")
            .unwrap();
        assert_eq!(test.duration_secs, Some(60.0));
        assert!(test.failed);
    }

    #[test]
    fn maps_artifact_sizes() {
        let run = parse_run(bundle()).unwrap();
        assert_eq!(run.artifacts[0].name, "build_loom/loom");
        assert_eq!(run.artifacts[0].size_bytes, 5_242_880);
    }

    #[test]
    fn end_to_end_into_profile() {
        let reports = GithubCollector.parse(&[RawRun(bundle().into())]).unwrap();
        let p = crate::telemetry::aggregate(&reports, &GithubCollector.runner_pricing());
        assert_eq!(p.action_duration("build_loom"), Some(30.0));
        assert_eq!(p.artifact_size("build_loom/loom"), Some(5_242_880));
        assert_eq!(p.setup_times.get("ubuntu-latest").copied(), Some(4.0));
        // ubuntu-latest priced at $0.008/min = $0.0001333.../sec.
        let cost = p.runner_costs.get("ubuntu-latest").copied().unwrap();
        assert!((cost - 0.008 / 60.0).abs() < 1e-9);
    }
}

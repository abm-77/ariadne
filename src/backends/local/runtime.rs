use std::path::PathBuf;
use std::process::Command;

pub struct ContainerSpec {
    pub image: String,
    pub command: Vec<String>,
    pub env: Vec<(String, String)>,
    pub mounts: Vec<Mount>,
    pub workdir: Option<String>,
    pub remove_after: bool,
}

pub struct Mount {
    pub host: String,
    pub container: String,
    pub read_only: bool,
}

pub struct RunResult {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
pub struct RuntimeError(pub String);

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub trait ContainerRuntime: Send + Sync {
    fn name(&self) -> &str;
    fn pull(&self, image: &str) -> Result<(), RuntimeError>;
    fn run(&self, spec: &ContainerSpec) -> Result<RunResult, RuntimeError>;
}

struct OciRuntime {
    name: String,
    bin: PathBuf,
}

impl OciRuntime {
    fn pull(&self, image: &str) -> Result<(), RuntimeError> {
        // Skip the network round-trip (and registry rate limits) when the image
        // is already present locally.
        let present = Command::new(&self.bin)
            .args(["image", "exists", image])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if present {
            return Ok(());
        }
        let status = Command::new(&self.bin)
            .args(["pull", image])
            .status()
            .map_err(|e| RuntimeError(format!("failed to invoke {}: {e}", self.name)))?;
        if status.success() {
            Ok(())
        } else {
            Err(RuntimeError(format!("{} pull '{}' failed", self.name, image)))
        }
    }

    fn run(&self, spec: &ContainerSpec) -> Result<RunResult, RuntimeError> {
        let mut cmd = Command::new(&self.bin);
        cmd.arg("run");
        if spec.remove_after {
            cmd.arg("--rm");
        }
        if let Some(ref wd) = spec.workdir {
            cmd.args(["-w", wd]);
        }
        for (k, v) in &spec.env {
            cmd.args(["-e", &format!("{k}={v}")]);
        }
        for m in &spec.mounts {
            let ro = if m.read_only { ":ro" } else { "" };
            cmd.args(["-v", &format!("{}:{}{ro}", m.host, m.container)]);
        }
        cmd.arg(&spec.image);
        cmd.args(&spec.command);
        let out = cmd.output()
            .map_err(|e| RuntimeError(format!("failed to invoke {}: {e}", self.name)))?;
        Ok(RunResult {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    }
}

pub struct PodmanRuntime(OciRuntime);

impl PodmanRuntime {
    pub fn new(bin: PathBuf) -> Self {
        Self(OciRuntime { name: "podman".into(), bin })
    }
}

impl Default for PodmanRuntime {
    fn default() -> Self { Self::new(PathBuf::from("podman")) }
}

impl ContainerRuntime for PodmanRuntime {
    fn name(&self) -> &str { self.0.name.as_str() }
    fn pull(&self, image: &str) -> Result<(), RuntimeError> { self.0.pull(image) }
    fn run(&self, spec: &ContainerSpec) -> Result<RunResult, RuntimeError> { self.0.run(spec) }
}

pub struct DockerRuntime(OciRuntime);

impl DockerRuntime {
    pub fn new(bin: PathBuf) -> Self {
        Self(OciRuntime { name: "docker".into(), bin })
    }
}

impl Default for DockerRuntime {
    fn default() -> Self { Self::new(PathBuf::from("docker")) }
}

impl ContainerRuntime for DockerRuntime {
    fn name(&self) -> &str { self.0.name.as_str() }
    fn pull(&self, image: &str) -> Result<(), RuntimeError> { self.0.pull(image) }
    fn run(&self, spec: &ContainerSpec) -> Result<RunResult, RuntimeError> { self.0.run(spec) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn podman_runtime_name() {
        assert_eq!(PodmanRuntime::default().name(), "podman");
    }

    #[test]
    fn docker_runtime_name() {
        assert_eq!(DockerRuntime::default().name(), "docker");
    }

}

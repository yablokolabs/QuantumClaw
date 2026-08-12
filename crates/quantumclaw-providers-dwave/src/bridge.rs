//! Process bridge to the Ocean SDK.
//!
//! QuantumClaw's core is Rust and Ocean is Python, so the boundary is a short
//! lived child process exchanging one JSON request and one JSON response. The
//! bridge owns every detail of that exchange: spawning, timeouts, protocol
//! errors, and translating the bridge's error codes into typed errors.

use crate::error::{DWaveError, Result};
use crate::models::{BridgeRequest, BridgeResponse, BridgeResult, ProbeReport};
use crate::DWaveConfig;
use std::process::Stdio;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

/// The bridge is not published to PyPI yet, so the actionable instruction is a
/// direct install from the repository.
const INSTALL_HINT: &str = "D-Wave Ocean backend is not installed. Install the QuantumClaw bridge with: pip install 'quantumclaw-dwave[dwave] @ git+https://github.com/yablokolabs/QuantumClaw#subdirectory=crates/quantumclaw-providers-dwave/python'";

/// Runs Ocean sampling requests through the Python bridge.
#[derive(Debug, Clone, Default)]
pub struct DWaveBridge {
    config: DWaveConfig,
}

/// Output of one bridge invocation, with the wall time the caller observed.
#[derive(Debug, Clone)]
pub struct BridgeExecution {
    pub result: BridgeResult,
    pub total_runtime_ms: u64,
}

impl DWaveBridge {
    pub fn new(config: DWaveConfig) -> Self {
        Self { config }
    }

    /// Builds a bridge from `QUANTUMCLAW_DWAVE_*` environment variables.
    pub fn from_env() -> Self {
        Self::new(DWaveConfig::from_env())
    }

    pub fn config(&self) -> &DWaveConfig {
        &self.config
    }

    /// Asks the bridge which Ocean components are installed.
    ///
    /// Callers use this for capability checks and to decide whether a D-Wave
    /// backend should be offered at all.
    pub async fn probe(&self) -> Result<ProbeReport> {
        let raw = self.invoke(None, &["--probe".to_string()]).await?;
        self.parse(&raw)
    }

    /// Runs one sampling request.
    pub async fn execute(&self, request: &BridgeRequest) -> Result<BridgeExecution> {
        let payload = serde_json::to_vec(request).map_err(|error| DWaveError::InvalidBqm {
            message: "the compiled model could not be serialized for the bridge".into(),
            cause: error.to_string(),
        })?;

        let started = Instant::now();
        let raw = self.invoke(Some(payload), &[]).await?;
        let total_runtime_ms = started.elapsed().as_millis() as u64;

        let result: BridgeResult = self.parse(&raw)?;

        Ok(BridgeExecution {
            result,
            total_runtime_ms,
        })
    }

    async fn invoke(
        &self,
        stdin_payload: Option<Vec<u8>>,
        extra_args: &[String],
    ) -> Result<RawOutput> {
        let mut command = Command::new(&self.config.python);
        command
            .args(&self.config.bridge_args)
            .args(extra_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if !self.config.python_path.is_empty() {
            let mut entries = self.config.python_path.clone();
            if let Some(existing) = std::env::var_os("PYTHONPATH") {
                entries.extend(std::env::split_paths(&existing));
            }
            let joined = std::env::join_paths(entries).map_err(|error| {
                DWaveError::InvalidConfiguration {
                    message: "the configured PYTHONPATH entries are not usable".into(),
                    cause: error.to_string(),
                }
            })?;
            command.env("PYTHONPATH", joined);
        }

        let mut child = command.spawn().map_err(|error| {
            let command_line = self.config.command_line();
            if error.kind() == std::io::ErrorKind::NotFound {
                DWaveError::OceanUnavailable {
                    message: format!(
                        "{INSTALL_HINT} (interpreter '{}' not found)",
                        self.config.python
                    ),
                    cause: error.to_string(),
                }
            } else {
                DWaveError::BridgeSpawn {
                    command: command_line,
                    cause: error.to_string(),
                }
            }
        })?;

        if let Some(mut stdin) = child.stdin.take() {
            let payload = stdin_payload.unwrap_or_default();
            stdin
                .write_all(&payload)
                .await
                .map_err(|error| DWaveError::BridgeSpawn {
                    command: self.config.command_line(),
                    cause: format!("could not write the request: {error}"),
                })?;
            stdin.shutdown().await.ok();
        }

        let output = match timeout(self.config.timeout, child.wait_with_output()).await {
            Ok(result) => result.map_err(|error| DWaveError::BridgeSpawn {
                command: self.config.command_line(),
                cause: error.to_string(),
            })?,
            Err(_) => {
                return Err(DWaveError::Timeout {
                    elapsed_ms: self.config.timeout.as_millis() as u64,
                    command: self.config.command_line(),
                })
            }
        };

        Ok(RawOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// Turns raw process output into either a decoded payload or a typed error.
    fn parse<T: serde::de::DeserializeOwned>(&self, raw: &RawOutput) -> Result<T> {
        let response: BridgeResponse =
            serde_json::from_str(raw.stdout.trim()).map_err(|error| {
                if looks_like_missing_module(&raw.stderr) {
                    DWaveError::OceanUnavailable {
                        message: format!(
                            "{INSTALL_HINT} (bridge module not importable by '{}')",
                            self.config.python
                        ),
                        cause: first_line(&raw.stderr),
                    }
                } else {
                    DWaveError::BridgeProtocol {
                        message: format!("stdout was not a bridge response: {error}"),
                        stdout: raw.stdout.clone(),
                        stderr: raw.stderr.clone(),
                    }
                }
            })?;

        if response.ok {
            let result = response.result.ok_or_else(|| DWaveError::BridgeProtocol {
                message: "a successful response carried no result".into(),
                stdout: raw.stdout.clone(),
                stderr: raw.stderr.clone(),
            })?;
            return serde_json::from_value(result).map_err(|error| DWaveError::BridgeProtocol {
                message: format!("the bridge payload could not be read: {error}"),
                stdout: raw.stdout.clone(),
                stderr: raw.stderr.clone(),
            });
        }

        let error = response.error.ok_or_else(|| DWaveError::BridgeProtocol {
            message: "a failed response carried no error".into(),
            stdout: raw.stdout.clone(),
            stderr: raw.stderr.clone(),
        })?;
        Err(DWaveError::from_bridge(
            &error.code,
            error.message,
            error.cause,
        ))
    }
}

#[derive(Debug, Clone)]
struct RawOutput {
    stdout: String,
    stderr: String,
}

fn looks_like_missing_module(stderr: &str) -> bool {
    stderr.contains("No module named 'quantumclaw_dwave'")
        || stderr.contains("No module named quantumclaw_dwave")
        || stderr.contains("No module named 'dimod'")
}

fn first_line(value: &str) -> String {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

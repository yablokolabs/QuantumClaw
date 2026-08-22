//! Provider configuration.
//!
//! Two independent concerns live here: how to reach the Python bridge, and how
//! to reach D-Wave Leap. The Leap API token is deliberately *not* one of them —
//! see [`LeapConfig`].

use std::env;
use std::path::PathBuf;
use std::time::Duration;

/// Environment variable holding the interpreter that runs the bridge.
pub const ENV_PYTHON: &str = "QUANTUMCLAW_DWAVE_PYTHON";
/// Environment variable holding an explicit bridge script path.
pub const ENV_BRIDGE: &str = "QUANTUMCLAW_DWAVE_BRIDGE";
/// Environment variable holding extra `PYTHONPATH` entries for the bridge.
pub const ENV_PYTHONPATH: &str = "QUANTUMCLAW_DWAVE_PYTHONPATH";
/// Environment variable holding the bridge timeout in milliseconds.
pub const ENV_TIMEOUT_MS: &str = "QUANTUMCLAW_DWAVE_TIMEOUT_MS";

/// How QuantumClaw reaches the Ocean bridge process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DWaveConfig {
    /// Python interpreter to run. Defaults to `python3`.
    pub python: String,
    /// Arguments that invoke the bridge. Defaults to the installed module.
    pub bridge_args: Vec<String>,
    /// Extra `PYTHONPATH` entries, used when running from a source checkout.
    pub python_path: Vec<PathBuf>,
    /// How long a single solve may take before the child is killed.
    pub timeout: Duration,
}

impl Default for DWaveConfig {
    fn default() -> Self {
        Self {
            python: "python3".into(),
            bridge_args: vec!["-m".into(), "quantumclaw_dwave.bridge".into()],
            python_path: Vec::new(),
            timeout: Duration::from_secs(120),
        }
    }
}

impl DWaveConfig {
    /// Reads configuration from the environment, falling back to defaults.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(python) = env::var(ENV_PYTHON) {
            if !python.trim().is_empty() {
                config.python = python;
            }
        }
        if let Ok(script) = env::var(ENV_BRIDGE) {
            if !script.trim().is_empty() {
                config.bridge_args = vec![script];
            }
        }
        if let Ok(paths) = env::var(ENV_PYTHONPATH) {
            config.python_path = env::split_paths(&paths).collect();
        }
        if let Ok(timeout) = env::var(ENV_TIMEOUT_MS) {
            if let Ok(millis) = timeout.parse::<u64>() {
                config.timeout = Duration::from_millis(millis);
            }
        }
        config
    }

    pub fn with_python(mut self, python: impl Into<String>) -> Self {
        self.python = python.into();
        self
    }

    pub fn with_bridge_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.bridge_args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_python_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.python_path.push(path.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Human-readable command, used in error messages.
    pub fn command_line(&self) -> String {
        std::iter::once(self.python.clone())
            .chain(self.bridge_args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Connection settings for D-Wave Leap.
///
/// The API token is intentionally absent. Ocean reads `DWAVE_API_TOKEN` or the
/// user's `dwave.conf` inside the bridge process, so the secret never enters
/// QuantumClaw's address space, its logs, or a process argument list. This type
/// only records whether a credential is *present*, so backends can fail early
/// with a useful message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeapConfig {
    pub solver: Option<String>,
    pub region: Option<String>,
    pub endpoint: Option<String>,
    pub profile: Option<String>,
}

impl LeapConfig {
    /// Reads the non-secret Leap settings from the standard D-Wave variables.
    pub fn from_env() -> Self {
        Self {
            solver: non_empty("DWAVE_SOLVER"),
            region: non_empty("DWAVE_REGION"),
            endpoint: non_empty("DWAVE_ENDPOINT"),
            profile: non_empty("DWAVE_PROFILE"),
        }
    }

    pub fn with_solver(mut self, solver: impl Into<String>) -> Self {
        self.solver = Some(solver.into());
        self
    }

    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Whether a Leap API token is visible in this process's environment.
    ///
    /// A `false` result does not prove the absence of credentials: Ocean also
    /// reads `~/.config/dwave/dwave.conf`, which only the bridge can see.
    pub fn token_in_environment() -> bool {
        non_empty("DWAVE_API_TOKEN").is_some()
    }
}

fn non_empty(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Parameters for the classical simulated annealing sampler.
#[derive(Debug, Clone, PartialEq)]
pub struct SimulatedAnnealingParams {
    pub num_reads: u32,
    pub num_sweeps: Option<u32>,
    pub beta_range: Option<(f64, f64)>,
    pub seed: Option<u64>,
}

impl Default for SimulatedAnnealingParams {
    fn default() -> Self {
        Self {
            num_reads: 100,
            num_sweeps: Some(1_000),
            beta_range: None,
            seed: None,
        }
    }
}

impl SimulatedAnnealingParams {
    pub fn with_num_reads(mut self, num_reads: u32) -> Self {
        self.num_reads = num_reads;
        self
    }

    pub fn with_num_sweeps(mut self, num_sweeps: u32) -> Self {
        self.num_sweeps = Some(num_sweeps);
        self
    }

    pub fn with_beta_range(mut self, low: f64, high: f64) -> Self {
        self.beta_range = Some((low, high));
        self
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }
}

/// Parameters for the local emulation of quantum annealing.
#[derive(Debug, Clone, PartialEq)]
pub struct SimulatedQuantumAnnealingParams {
    pub num_reads: u32,
    pub num_sweeps: Option<u32>,
    pub beta_range: Option<(f64, f64)>,
    pub seed: Option<u64>,
}

impl Default for SimulatedQuantumAnnealingParams {
    fn default() -> Self {
        Self {
            num_reads: 100,
            num_sweeps: Some(1_000),
            beta_range: None,
            seed: None,
        }
    }
}

impl SimulatedQuantumAnnealingParams {
    pub fn with_num_reads(mut self, num_reads: u32) -> Self {
        self.num_reads = num_reads;
        self
    }

    pub fn with_num_sweeps(mut self, num_sweeps: u32) -> Self {
        self.num_sweeps = Some(num_sweeps);
        self
    }

    pub fn with_beta_range(mut self, low: f64, high: f64) -> Self {
        self.beta_range = Some((low, high));
        self
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }
}

/// Parameters for the exhaustive classical solver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactParams {
    /// Refuse problems above this many variables. Exhaustive search evaluates
    /// `2^n` assignments, so this guard is not optional in practice.
    pub max_variables: usize,
}

impl Default for ExactParams {
    fn default() -> Self {
        Self { max_variables: 20 }
    }
}

impl ExactParams {
    pub fn with_max_variables(mut self, max_variables: usize) -> Self {
        self.max_variables = max_variables;
        self
    }
}

/// Parameters for the Leap hybrid solver.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HybridParams {
    pub time_limit_s: Option<f64>,
    pub label: Option<String>,
}

impl HybridParams {
    pub fn with_time_limit_s(mut self, time_limit_s: f64) -> Self {
        self.time_limit_s = Some(time_limit_s);
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

/// Parameters for quantum annealing hardware.
#[derive(Debug, Clone, PartialEq)]
pub struct QpuParams {
    pub num_reads: u32,
    pub chain_strength: Option<f64>,
    pub annealing_time_us: Option<f64>,
    pub label: Option<String>,
    /// Largest problem this backend will attempt to embed. Embedding cost grows
    /// quickly, so a guard avoids long failures on hardware time.
    pub max_variables: usize,
}

impl Default for QpuParams {
    fn default() -> Self {
        Self {
            num_reads: 100,
            chain_strength: None,
            annealing_time_us: None,
            label: None,
            max_variables: 5_000,
        }
    }
}

impl QpuParams {
    pub fn with_num_reads(mut self, num_reads: u32) -> Self {
        self.num_reads = num_reads;
        self
    }

    pub fn with_chain_strength(mut self, chain_strength: f64) -> Self {
        self.chain_strength = Some(chain_strength);
        self
    }

    pub fn with_annealing_time_us(mut self, annealing_time_us: f64) -> Self {
        self.annealing_time_us = Some(annealing_time_us);
        self
    }

    pub fn with_max_variables(mut self, max_variables: usize) -> Self {
        self.max_variables = max_variables;
        self
    }
}

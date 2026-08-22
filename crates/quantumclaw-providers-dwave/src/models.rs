//! Wire format shared with the Python bridge.
//!
//! These types exist only at the provider boundary. Nothing in the QuantumClaw
//! core or in any domain brain sees them.

use quantumclaw_ir::optimization::BinaryQuadraticModel;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const PROTOCOL_VERSION: u32 = 1;

/// Which Ocean lane the bridge should run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeBackend {
    SimulatedAnnealing,
    Exact,
    Hybrid,
    Qpu,
    SimulatedQuantumAnnealing,
}

impl BridgeBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SimulatedAnnealing => "simulated_annealing",
            Self::Exact => "exact",
            Self::Hybrid => "hybrid",
            Self::Qpu => "qpu",
            Self::SimulatedQuantumAnnealing => "simulated_quantum_annealing",
        }
    }
}

/// A binary quadratic model in the bridge's compact array encoding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeBqm {
    pub variables: Vec<String>,
    pub linear: Vec<(String, f64)>,
    pub quadratic: Vec<(String, String, f64)>,
    pub offset: f64,
}

impl From<&BinaryQuadraticModel> for BridgeBqm {
    fn from(value: &BinaryQuadraticModel) -> Self {
        Self {
            variables: value.variables.clone(),
            linear: value
                .linear
                .iter()
                .map(|term| (term.variable.clone(), term.coefficient))
                .collect(),
            quadratic: value
                .quadratic
                .iter()
                .map(|term| (term.first.clone(), term.second.clone(), term.coefficient))
                .collect(),
            offset: value.offset,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BridgeRequest {
    pub protocol_version: u32,
    pub backend: &'static str,
    pub problem_id: String,
    pub bqm: BridgeBqm,
    pub parameters: BTreeMap<String, Value>,
    pub options: BTreeMap<String, Value>,
}

impl BridgeRequest {
    pub fn new(backend: BridgeBackend, problem_id: impl Into<String>, bqm: BridgeBqm) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            backend: backend.as_str(),
            problem_id: problem_id.into(),
            bqm,
            parameters: BTreeMap::new(),
            options: BTreeMap::new(),
        }
    }

    pub fn with_parameter(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.parameters.insert(key.into(), value.into());
        self
    }

    pub fn with_optional_parameter<T: Into<Value>>(self, key: &str, value: Option<T>) -> Self {
        match value {
            Some(value) => self.with_parameter(key, value),
            None => self,
        }
    }

    pub fn with_option(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.options.insert(key.into(), value.into());
        self
    }

    pub fn with_optional_option<T: Into<Value>>(self, key: &str, value: Option<T>) -> Self {
        match value {
            Some(value) => self.with_option(key, value),
            None => self,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BridgeResponse {
    pub ok: bool,
    /// Left undecoded so one envelope serves both sampling results and probes.
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<BridgeErrorPayload>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BridgeErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub cause: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BridgeSample {
    pub sample: BTreeMap<String, u8>,
    pub energy: f64,
    #[serde(default)]
    pub num_occurrences: u64,
}

/// What the bridge reports after a successful run. Every measurement that a
/// given sampler cannot produce is simply absent.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BridgeResult {
    pub backend: String,
    pub sampler: String,
    pub problem_type: String,
    pub num_variables: usize,
    pub num_interactions: usize,
    pub best: BridgeSample,
    #[serde(default)]
    pub num_samples: usize,
    #[serde(default)]
    pub num_reads: Option<u32>,
    #[serde(default)]
    pub solver_runtime_ms: Option<f64>,
    #[serde(default)]
    pub qpu_access_time_us: Option<f64>,
    #[serde(default)]
    pub run_time_us: Option<f64>,
    #[serde(default)]
    pub charge_time_us: Option<f64>,
    #[serde(default)]
    pub chain_break_fraction: Option<f64>,
    #[serde(default)]
    pub info: Value,
}

/// Availability report produced by `python -m quantumclaw_dwave.bridge --probe`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProbeReport {
    #[serde(default)]
    pub available: BTreeMap<String, bool>,
    #[serde(default)]
    pub versions: BTreeMap<String, String>,
    #[serde(default)]
    pub backends: BTreeMap<String, bool>,
    #[serde(default)]
    pub credentials_present: bool,
}

impl ProbeReport {
    pub fn supports(&self, backend: BridgeBackend) -> bool {
        self.backends
            .get(backend.as_str())
            .copied()
            .unwrap_or(false)
    }
}

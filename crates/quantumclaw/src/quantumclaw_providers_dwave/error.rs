use crate::quantumclaw_core::QuantumClawError;
use crate::quantumclaw_optimization::OptimizationError;
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Failures raised by the D-Wave provider.
///
/// Provider failures are never swallowed: the message reported by Ocean is
/// preserved in `cause`, and the typed variant tells callers what class of
/// failure it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DWaveError {
    /// The Ocean SDK is not importable on the configured interpreter.
    OceanUnavailable { message: String, cause: String },
    /// The bridge process could not be started at all.
    BridgeSpawn { command: String, cause: String },
    /// The bridge produced something that is not a valid response.
    BridgeProtocol {
        message: String,
        stdout: String,
        stderr: String,
    },
    /// The bridge did not finish within the configured timeout.
    Timeout { elapsed_ms: u64, command: String },
    /// The compiled model was rejected by the bridge.
    InvalidBqm { message: String, cause: String },
    /// Backend parameters or provider settings are not usable.
    InvalidConfiguration { message: String, cause: String },
    /// Leap credentials are absent.
    MissingCredentials { message: String, cause: String },
    /// Leap rejected the supplied credentials.
    Authentication { message: String, cause: String },
    /// The requested solver is offline, unknown, or unreachable.
    SolverUnavailable { message: String, cause: String },
    /// The problem could not be embedded onto the QPU topology.
    EmbeddingFailed { message: String, cause: String },
    /// The sampler returned nothing usable.
    NoFeasibleResult { message: String, cause: String },
    /// The problem exceeds what this backend accepts.
    ProblemTooLarge { message: String, cause: String },
    /// The sampler failed for a provider-specific reason.
    SamplerFailed { message: String, cause: String },
    /// The decision problem could not be turned into a binary model.
    Compilation(OptimizationError),
    /// The backend cannot accept this problem.
    UnsupportedProblem { message: String },
}

impl DWaveError {
    /// Stable machine-readable code, shared with the Python bridge.
    pub fn code(&self) -> &'static str {
        match self {
            Self::OceanUnavailable { .. } => "ocean_missing",
            Self::BridgeSpawn { .. } => "bridge_spawn_failed",
            Self::BridgeProtocol { .. } => "bridge_protocol_error",
            Self::Timeout { .. } => "timeout",
            Self::InvalidBqm { .. } => "invalid_bqm",
            Self::InvalidConfiguration { .. } => "invalid_configuration",
            Self::MissingCredentials { .. } => "missing_credentials",
            Self::Authentication { .. } => "authentication_failed",
            Self::SolverUnavailable { .. } => "solver_unavailable",
            Self::EmbeddingFailed { .. } => "embedding_failed",
            Self::NoFeasibleResult { .. } => "no_feasible_result",
            Self::ProblemTooLarge { .. } => "problem_too_large",
            Self::SamplerFailed { .. } => "sampler_failed",
            Self::Compilation(_) => "compilation_failed",
            Self::UnsupportedProblem { .. } => "unsupported_problem",
        }
    }

    /// Builds the typed error for a code reported by the bridge.
    pub fn from_bridge(code: &str, message: String, cause: Option<String>) -> Self {
        let cause = cause.unwrap_or_default();
        match code {
            "ocean_missing" => Self::OceanUnavailable { message, cause },
            "invalid_bqm" => Self::InvalidBqm { message, cause },
            "invalid_request" | "invalid_configuration" => {
                Self::InvalidConfiguration { message, cause }
            }
            "missing_credentials" => Self::MissingCredentials { message, cause },
            "authentication_failed" => Self::Authentication { message, cause },
            "solver_unavailable" => Self::SolverUnavailable { message, cause },
            "embedding_failed" => Self::EmbeddingFailed { message, cause },
            "timeout" => Self::Timeout {
                elapsed_ms: 0,
                command: message,
            },
            "no_feasible_result" => Self::NoFeasibleResult { message, cause },
            "problem_too_large" => Self::ProblemTooLarge { message, cause },
            _ => Self::SamplerFailed { message, cause },
        }
    }

    fn parts(&self) -> (&str, &str) {
        match self {
            Self::OceanUnavailable { message, cause }
            | Self::InvalidBqm { message, cause }
            | Self::InvalidConfiguration { message, cause }
            | Self::MissingCredentials { message, cause }
            | Self::Authentication { message, cause }
            | Self::SolverUnavailable { message, cause }
            | Self::EmbeddingFailed { message, cause }
            | Self::NoFeasibleResult { message, cause }
            | Self::ProblemTooLarge { message, cause }
            | Self::SamplerFailed { message, cause } => (message, cause),
            _ => ("", ""),
        }
    }
}

impl Display for DWaveError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BridgeSpawn { command, cause } => {
                write!(f, "could not start the D-Wave bridge '{command}': {cause}")
            }
            Self::BridgeProtocol {
                message,
                stdout,
                stderr,
            } => {
                write!(
                    f,
                    "the D-Wave bridge returned an unusable response: {message}"
                )?;
                if !stdout.trim().is_empty() {
                    write!(f, " (stdout: {})", truncate(stdout))?;
                }
                if !stderr.trim().is_empty() {
                    write!(f, " (stderr: {})", truncate(stderr))?;
                }
                Ok(())
            }
            Self::Timeout {
                elapsed_ms,
                command,
            } => write!(
                f,
                "the D-Wave bridge '{command}' did not finish within {elapsed_ms}ms"
            ),
            Self::Compilation(error) => write!(f, "{error}"),
            Self::UnsupportedProblem { message } => write!(f, "{message}"),
            _ => {
                let (message, cause) = self.parts();
                write!(f, "{message}")?;
                if !cause.is_empty() {
                    write!(f, " (cause: {cause})")?;
                }
                Ok(())
            }
        }
    }
}

impl Error for DWaveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Compilation(error) => Some(error),
            _ => None,
        }
    }
}

impl From<OptimizationError> for DWaveError {
    fn from(value: OptimizationError) -> Self {
        Self::Compilation(value)
    }
}

impl From<DWaveError> for QuantumClawError {
    fn from(value: DWaveError) -> Self {
        QuantumClawError::new(format!("dwave provider [{}]: {value}", value.code()))
    }
}

fn truncate(value: &str) -> String {
    const LIMIT: usize = 400;
    let trimmed = value.trim();
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(LIMIT).collect();
    format!("{head}…")
}

pub type Result<T> = std::result::Result<T, DWaveError>;

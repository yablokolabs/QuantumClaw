use crate::quantumclaw_core::QuantumClawError;
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Failures raised while compiling or decoding an optimization problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptimizationError {
    /// The problem declares no variables, so there is nothing to solve.
    EmptyProblem { problem_id: String },
    /// A constraint references a variable that the problem never declared.
    UnknownVariable {
        constraint_id: String,
        variable: String,
    },
    /// The constraint cannot be expressed as a quadratic penalty.
    UnsupportedConstraint {
        constraint_id: String,
        reason: String,
    },
    /// A penalty weight was zero, negative, or not finite.
    InvalidPenalty {
        constraint_id: String,
        reason: String,
    },
    /// Slack encoding for an inequality would need an unreasonable number of
    /// auxiliary variables.
    SlackOverflow {
        constraint_id: String,
        required_bits: u32,
        max_bits: u32,
    },
    /// The decision problem carries nothing that can be optimized.
    UnsupportedDecisionProblem { problem_id: String, reason: String },
    /// An exhaustive search was requested on a problem that is too large.
    ProblemTooLarge { variables: usize, limit: usize },
}

impl Display for OptimizationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyProblem { problem_id } => {
                write!(f, "optimization problem '{problem_id}' declares no variables")
            }
            Self::UnknownVariable {
                constraint_id,
                variable,
            } => write!(
                f,
                "constraint '{constraint_id}' references undeclared variable '{variable}'"
            ),
            Self::UnsupportedConstraint {
                constraint_id,
                reason,
            } => write!(
                f,
                "constraint '{constraint_id}' cannot be compiled into a QUBO: {reason}"
            ),
            Self::InvalidPenalty {
                constraint_id,
                reason,
            } => write!(f, "constraint '{constraint_id}' has an invalid penalty: {reason}"),
            Self::SlackOverflow {
                constraint_id,
                required_bits,
                max_bits,
            } => write!(
                f,
                "constraint '{constraint_id}' needs {required_bits} slack bits but the compiler allows {max_bits}"
            ),
            Self::UnsupportedDecisionProblem { problem_id, reason } => write!(
                f,
                "decision problem '{problem_id}' cannot be optimized: {reason}"
            ),
            Self::ProblemTooLarge { variables, limit } => write!(
                f,
                "exhaustive search needs at most {limit} variables but the model has {variables}"
            ),
        }
    }
}

impl Error for OptimizationError {}

impl From<OptimizationError> for QuantumClawError {
    fn from(value: OptimizationError) -> Self {
        QuantumClawError::new(value.to_string())
    }
}

pub type Result<T> = std::result::Result<T, OptimizationError>;

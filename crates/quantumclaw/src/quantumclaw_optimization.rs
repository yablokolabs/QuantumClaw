//! Domain-neutral optimization layer.
//!
//! This crate turns a [`crate::quantumclaw_ir::OptimizationProblem`] into a
//! [`crate::quantumclaw_ir::BinaryQuadraticModel`] in minimization form, and turns
//! samples returned by any solver back into a normalized
//! [`crate::quantumclaw_ir::OptimizationSolution`].
//!
//! Nothing here knows about routing, scheduling, or any solver provider. Domain
//! brains build [`crate::quantumclaw_ir::OptimizationProblem`]s; solver backends
//! consume [`CompiledModel`]s.

pub mod compiler;
pub mod decision;
pub mod error;

pub use compiler::{CompiledModel, QuboCompiler, SlackVariable};
pub use decision::{action_selection_problem, optimization_problem_for};
pub use error::{OptimizationError, Result};

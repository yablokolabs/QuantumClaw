//! Domain-neutral optimization layer.
//!
//! This crate turns a [`quantumclaw_ir::OptimizationProblem`] into a
//! [`quantumclaw_ir::BinaryQuadraticModel`] in minimization form, and turns
//! samples returned by any solver back into a normalized
//! [`quantumclaw_ir::OptimizationSolution`].
//!
//! Nothing here knows about routing, scheduling, or any solver provider. Domain
//! brains build [`quantumclaw_ir::OptimizationProblem`]s; solver backends
//! consume [`CompiledModel`]s.

pub mod compiler;
pub mod decision;
pub mod error;

pub use compiler::{CompiledModel, QuboCompiler, SlackVariable};
pub use decision::{action_selection_problem, optimization_problem_for};
pub use error::{OptimizationError, Result};

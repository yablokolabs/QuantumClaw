//! Umbrella crate for QuantumClaw.
//!
//! QuantumClaw is a ZeroClaw-backed agent runtime with backend-neutral planning
//! traits, classical solvers, quantum-inspired solver scaffolds, and optional
//! future QPU adapter boundaries. This crate re-exports the workspace crates so
//! consumers can depend on a single `quantumclaw` package and opt into modules as
//! the runtime evolves.

pub use quantumclaw_core as core;
pub use quantumclaw_ir as ir;
pub use quantumclaw_memory as memory;
pub use quantumclaw_observability as observability;
pub use quantumclaw_planner as planner;
pub use quantumclaw_policy as policy;
pub use quantumclaw_runtime as runtime;
pub use quantumclaw_skills as skills;
pub use quantumclaw_solvers_classical as solvers_classical;
pub use quantumclaw_solvers_future_qpu as solvers_future_qpu;
pub use quantumclaw_solvers_qinspired as solvers_qinspired;
pub use quantumclaw_tools as tools;

/// Common imports for building QuantumClaw runtimes and planners.
pub mod prelude {
    pub use crate::{
        core, ir, memory, observability, planner, policy, runtime, skills, solvers_classical,
        solvers_future_qpu, solvers_qinspired, tools,
    };
}

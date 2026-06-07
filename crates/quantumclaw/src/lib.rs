//! Single public crate for QuantumClaw.
//!
//! Consumers should depend on this `quantumclaw` package only. The workspace
//! component crates remain private implementation crates; their public APIs are
//! mirrored inside this package and re-exported from the crate root.

// The component APIs intentionally expose some overlapping domain names
// (for example telemetry types). Keep the requested single-crate glob surface
// without making ambiguous re-export warnings fail CI.
#![allow(ambiguous_glob_reexports)]

pub mod quantumclaw_core;
pub mod quantumclaw_ir;
pub mod quantumclaw_memory;
pub mod quantumclaw_observability;
pub mod quantumclaw_planner;
pub mod quantumclaw_policy;
pub mod quantumclaw_runtime;
pub mod quantumclaw_skills;
pub mod quantumclaw_tools;

pub use quantumclaw_core::*;
pub use quantumclaw_ir::*;
pub use quantumclaw_memory::*;
pub use quantumclaw_observability::*;
pub use quantumclaw_planner::*;
pub use quantumclaw_policy::*;
pub use quantumclaw_runtime::*;
pub use quantumclaw_skills::*;
pub use quantumclaw_tools::*;

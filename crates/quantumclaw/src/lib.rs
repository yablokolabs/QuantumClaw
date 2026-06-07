//! Single public crate for QuantumClaw.
//!
//! End users should depend on this `quantumclaw` package only. Internal
//! workspace crates stay private to the repository and are mirrored here behind
//! the single public product API.

// Some component APIs intentionally expose overlapping domain names
// (for example telemetry types). Keep root-level compatibility re-exports
// while guiding users toward `prelude` and short module paths.
#![allow(ambiguous_glob_reexports)]

#[doc(hidden)]
pub mod quantumclaw_core;
#[doc(hidden)]
pub mod quantumclaw_ir;
#[doc(hidden)]
pub mod quantumclaw_memory;
#[doc(hidden)]
pub mod quantumclaw_observability;
#[doc(hidden)]
pub mod quantumclaw_planner;
#[doc(hidden)]
pub mod quantumclaw_policy;
#[doc(hidden)]
pub mod quantumclaw_runtime;
#[doc(hidden)]
pub mod quantumclaw_skills;
#[doc(hidden)]
pub mod quantumclaw_solvers_classical;
#[doc(hidden)]
pub mod quantumclaw_solvers_future_qpu;
#[doc(hidden)]
pub mod quantumclaw_solvers_qinspired;
#[doc(hidden)]
pub mod quantumclaw_tools;

/// Core shared traits and runtime primitives.
pub mod core {
    pub use crate::quantumclaw_core::*;
}

/// Backend-neutral decision IR.
pub mod ir {
    pub use crate::quantumclaw_ir::*;
}

/// Working, short-term, episodic, semantic, and procedural memory APIs.
pub mod memory {
    pub use crate::quantumclaw_memory::*;
}

/// Trace, metric, planner comparison, and audit APIs.
pub mod observability {
    pub use crate::quantumclaw_observability::*;
}

/// Planner modes, requests, responses, and hybrid planning orchestration.
pub mod planner {
    pub use crate::quantumclaw_planner::*;
}

/// Deterministic policy, permission, risk, and audit controls.
pub mod policy {
    pub use crate::quantumclaw_policy::*;
}

/// ZeroClaw-backed runtime orchestration.
pub mod runtime {
    pub use crate::quantumclaw_runtime::*;
}

/// Procedural skill and recipe APIs.
pub mod skills {
    pub use crate::quantumclaw_skills::*;
}

/// Policy-controlled tool registry and tool-call APIs.
pub mod tools {
    pub use crate::quantumclaw_tools::*;
}

/// Solver backends shipped inside the single public `quantumclaw` crate.
pub mod solvers {
    /// Classical search and optimization solver backends.
    pub mod classical {
        pub use crate::quantumclaw_solvers_classical::*;
    }

    /// Future QPU adapter boundaries.
    pub mod future_qpu {
        pub use crate::quantumclaw_solvers_future_qpu::*;
    }

    /// Quantum-inspired solver scaffolds and mappings.
    pub mod qinspired {
        pub use crate::quantumclaw_solvers_qinspired::*;
    }
}

/// Common imports for most QuantumClaw applications.
pub mod prelude {
    pub use crate::core::*;
    pub use crate::ir::*;
    pub use crate::memory::*;
    pub use crate::observability::{
        AuditSink, ExecutionTrace, InMemoryObserver, MetricEvent, PlannerComparisonEvent,
        TraceEvent,
    };
    pub use crate::planner::*;
    pub use crate::policy::{
        AuditEvent, DeterministicPolicyEngine, DomainPolicyPack, HumanConfirmationThreshold,
        Permission, PlanAuditLog, PolicyDecision, RiskLevel,
    };
    pub use crate::runtime::{
        AgentLifecycle, Channel, InMemorySubagentRegistry, MessageRouter, PromptTemplate,
        ProviderAdapter, ProviderRequest, ProviderResponse, QuantumClawRuntime, RetryPolicy,
        RuntimeExecutionReport, RuntimeSession, SimulatedExecution, ZeroClawBase,
        ZeroClawExecutionContext, ZeroClawProceduralMemory,
    };
    pub use crate::skills::*;
    pub use crate::solvers::classical::{
        BeamSearchSolver, BranchAndBoundSolver, EvolutionarySolver, GreedySolver,
        HeuristicSearchSolver, SimulatedAnnealingSolver,
    };
    pub use crate::solvers::future_qpu::{
        FutureQpuBackend, FutureQpuJobHandle, FutureQpuSolverAdapter, PlaceholderFutureQpuBackend,
    };
    pub use crate::solvers::qinspired::{IsingLikeMapping, QuantumInspiredSolver, QuboLikeProblem};
    pub use crate::tools::{
        InMemoryToolRegistry, QuantumClawToolAdapter, ToolCall, ToolPermission, ToolResult,
        ToolSchema, ZeroClawToolAdapter, ZeroClawToolRef,
    };
}

// Root-level compatibility exports. Prefer `quantumclaw::prelude::*` for apps
// and `quantumclaw::{planner, runtime, memory, tools, policy}` for namespacing.
pub use quantumclaw_core::*;
pub use quantumclaw_ir::*;
pub use quantumclaw_memory::*;
pub use quantumclaw_observability::*;
pub use quantumclaw_planner::*;
pub use quantumclaw_policy::*;
pub use quantumclaw_runtime::*;
pub use quantumclaw_skills::*;
pub use quantumclaw_tools::*;

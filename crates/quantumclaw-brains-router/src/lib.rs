//! Q-Router: the logistics quantum brain.
//!
//! Q-Router owns routing knowledge — depots, heterogeneous fleets, capacities,
//! delivery windows, distance matrices, fuel, CO2, SLA penalties — and nothing
//! else. It expresses the combinatorial core of a routing problem as a
//! domain-neutral [`quantumclaw_ir::optimization::OptimizationProblem`] and
//! hands it to whatever [`quantumclaw_core::SolverBackend`] the registry
//! offers.
//!
//! Deliberate boundaries:
//!
//! * **No provider knowledge.** Nothing in this crate imports or names a solver
//!   provider. Backends arrive by name through a registry.
//! * **Quantum where it is defensible.** Vehicle assignment becomes a QUBO.
//!   Route sequencing stays classical, because a 10,000-stop tour is not a
//!   sensible QUBO and pretending otherwise would be dishonest.
//! * **Decomposition first.** Large instances are partitioned before anything
//!   is formulated, so no single model grows past what a solver can take.
//!
//! ```text
//! DeliveryProblem -> validate -> decompose -> assignment QUBO -> SolverBackend
//!                 -> decode -> repair -> sequence -> constraints -> KPIs
//! ```

pub mod benchmark;
pub mod brain;
pub mod compiler;
pub mod constraints;
pub mod decoder;
pub mod decomposition;
pub mod kpis;
pub mod models;
pub mod network;
pub mod routing_policy;
pub mod tools;
pub mod vrp;

pub use benchmark::{BenchmarkEntry, RouterBenchmark, RouterBenchmarkReport};
pub use brain::{QRouterBrain, QRouterRequest, QRouterResult, RouterOptions, SubproblemReport};
pub use compiler::{assignment_problem, AssignmentWeights};
pub use constraints::{RouteEvaluation, RouterViolation, ViolationKind};
pub use decomposition::{
    CapacityCluster, DecompositionPolicy, DecompositionStrategy, DepotPartition, GeographicCluster,
    RollingHorizon, SingleBlock, Subproblem, SubproblemClass, TimeWindowPartition,
};
pub use kpis::{KpiImprovement, RouterKpis};
pub use models::{
    Delivery, DeliveryProblem, Depot, DistanceMatrix, Location, Route, RouteSolution,
    RouterCostModel, SlaPolicy, TimeWindow, Vehicle,
};
pub use network::Network;
pub use routing_policy::{BenchmarkLedger, LedgerRecord, RoutingDecision, SolverRoutingPolicy};
pub use tools::{
    QRouterToolContext, TOOL_BENCHMARK, TOOL_COMPARE_SOLVERS, TOOL_OPTIMIZE, TOOL_VALIDATE,
};

use quantumclaw_brains::{BrainRegistry, JsonBrain};
use std::sync::Arc;

/// Registers Q-Router with a brain registry so agent tasks can route to it.
pub fn register_brain(registry: &mut BrainRegistry, brain: Arc<QRouterBrain>) {
    registry.register(Arc::new(JsonBrain::new(brain)));
}

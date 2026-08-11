//! Default wiring of solvers, brains, and tools.
//!
//! This is the one place that knows every component exists. Everything below it
//! — planner, brains, runtime — works through registries and never names a
//! provider.

use quantumclaw_brains::BrainRegistry;
use quantumclaw_brains_router::tools::tools as router_tools;
use quantumclaw_brains_router::{QRouterBrain, QRouterToolContext};
use quantumclaw_core::{Result, SolverRegistry, ToolRegistry};
use quantumclaw_providers_dwave::DWaveBridge;
use quantumclaw_solvers_classical::{
    BeamSearchSolver, BranchAndBoundSolver, EvolutionarySolver, GreedySolver,
    HeuristicSearchSolver, SimulatedAnnealingSolver,
};
use quantumclaw_solvers_qinspired::QuantumInspiredSolver;
use quantumclaw_tools::InMemoryToolRegistry;
use std::sync::Arc;

/// Every solver backend QuantumClaw ships, keyed by name.
///
/// D-Wave backends register unconditionally. Registration touches neither Ocean
/// nor the network; a missing dependency is reported at solve time with an
/// actionable message, so `--backend dwave-sa` always fails for the real reason
/// rather than "unknown backend".
pub fn solver_registry() -> SolverRegistry {
    let mut registry = SolverRegistry::new();
    registry.register(Arc::new(GreedySolver));
    registry.register(Arc::new(BeamSearchSolver::default()));
    registry.register(Arc::new(HeuristicSearchSolver));
    registry.register(Arc::new(BranchAndBoundSolver));
    registry.register(Arc::new(SimulatedAnnealingSolver));
    registry.register(Arc::new(EvolutionarySolver));
    registry.register(Arc::new(QuantumInspiredSolver::default()));
    quantumclaw_providers_dwave::register_backends(
        &mut registry,
        Arc::new(DWaveBridge::from_env()),
    );
    registry
}

/// Every domain brain, ready for intent routing.
pub fn brain_registry() -> BrainRegistry {
    let mut registry = BrainRegistry::new();
    quantumclaw_brains_router::register_brain(&mut registry, Arc::new(QRouterBrain::new()));
    registry
}

/// The default tool registry plus the Q-Router tools.
pub async fn tool_registry(solvers: Arc<SolverRegistry>) -> Result<InMemoryToolRegistry> {
    let registry = InMemoryToolRegistry::with_default_tools();
    let context = QRouterToolContext::new(Arc::new(QRouterBrain::new()), solvers);
    for tool in router_tools(context) {
        registry.register(tool).await?;
    }
    Ok(registry)
}

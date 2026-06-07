use std::{any::type_name, sync::Arc};

use quantumclaw::prelude::*;

#[test]
fn prelude_exposes_common_application_api() {
    let task = AgentTask::new("Plan a safe coding refactor");
    let _context = SolverContext::from_task(&task);
    let _planner = HybridPlanner::default();
    let _memory = InMemoryProceduralMemory::default();
    let _tools = InMemoryToolRegistry::with_default_tools();
    let _policy = DeterministicPolicyEngine::default();
    let _observer = InMemoryObserver::default();

    let _backends: Vec<Arc<dyn SolverBackend>> = vec![
        Arc::new(GreedySolver),
        Arc::new(QuantumInspiredSolver::default()),
    ];
}

#[test]
fn short_module_paths_expose_public_api() {
    let _ = type_name::<quantumclaw::planner::HybridPlanner>();
    let _ = type_name::<quantumclaw::runtime::QuantumClawRuntime>();
    let _ = type_name::<quantumclaw::memory::InMemoryProceduralMemory>();
    let _ = type_name::<quantumclaw::tools::InMemoryToolRegistry>();
    let _ = type_name::<quantumclaw::policy::DeterministicPolicyEngine>();
    let _ = type_name::<quantumclaw::solvers::classical::GreedySolver>();
    let _ = type_name::<quantumclaw::solvers::qinspired::QuantumInspiredSolver>();
}

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

#[test]
fn the_public_crate_exposes_the_optimization_layer() {
    use quantumclaw::ir::optimization::{OptimizationConstraint, OptimizationProblem};
    use quantumclaw::optimization::QuboCompiler;

    // Two mutually exclusive options; the cheaper one wins.
    let problem = OptimizationProblem::minimize("public-api")
        .with_term("cheap", 1.0)
        .with_term("expensive", 5.0)
        .with_constraint(OptimizationConstraint::exactly_one(
            "pick-one",
            ["cheap", "expensive"],
        ));

    let best = QuboCompiler::default()
        .compile(&problem)
        .expect("the model compiles")
        .brute_force()
        .expect("the model is small");

    assert_eq!(best.selected, vec!["cheap"]);
    assert!(best.feasible);
}

#[test]
fn the_public_crate_exposes_every_dwave_backend() {
    use quantumclaw::core::{SolverBackend, SolverKind, SolverRegistry};
    use quantumclaw::providers::dwave::{
        register_backends_from_env, DWaveSimulatedAnnealingBackend,
        DWaveSimulatedQuantumAnnealingBackend,
    };

    let mut registry = SolverRegistry::new();
    register_backends_from_env(&mut registry);

    for name in [
        "dwave-sa",
        "dwave-sqa",
        "dwave-exact",
        "dwave-hybrid",
        "dwave-qpu",
    ] {
        assert!(registry.get(name).is_some(), "{name} must be selectable");
    }
    // Simulated annealing is classical, whatever SDK drives it.
    assert_eq!(
        DWaveSimulatedAnnealingBackend::from_env().kind(),
        SolverKind::Classical
    );
    // The local emulator is quantum-inspired, never a quantum device.
    assert_eq!(
        DWaveSimulatedQuantumAnnealingBackend::from_env().kind(),
        SolverKind::QuantumInspired
    );
}

#[tokio::test]
async fn the_public_crate_exposes_the_qrouter_brain() {
    use quantumclaw::brains::router::{
        Delivery, DeliveryProblem, Depot, Location, QRouterBrain, QRouterRequest, Vehicle,
    };
    use quantumclaw::brains::{BrainSolveContext, QuantumBrain};

    let problem = DeliveryProblem::new("public-api")
        .with_depot(Depot::new("depot", Location::new(0.0, 0.0)))
        .with_vehicle(Vehicle::new("truck", "depot", 10))
        .with_delivery(Delivery::new("stop-1", Location::new(0.05, 0.05), 4))
        .with_delivery(Delivery::new("stop-2", Location::new(0.06, 0.06), 4));

    let result = QRouterBrain::new()
        .solve(QRouterRequest::new(problem), BrainSolveContext::default())
        .await
        .expect("the brain solves through the public API");

    assert_eq!(result.kpis.deliveries_served, 2);
    assert!(result.feasible);
}

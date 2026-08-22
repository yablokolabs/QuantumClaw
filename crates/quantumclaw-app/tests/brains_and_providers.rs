//! Architectural guarantees for the brain and provider layers.

use quantumclaw_app::{brain_registry, solver_registry, tool_registry};
use quantumclaw_brains::QuantumBrain;
use quantumclaw_brains::{BrainOperation, BrainSolveContext};
use quantumclaw_brains_router::brain::{QRouterBrain, QRouterRequest};
use quantumclaw_brains_router::models::{
    Delivery, DeliveryProblem, Depot, DistanceMatrix, Location, Vehicle,
};
use quantumclaw_core::{AgentTask, CoreToolCall, SolverKind, ToolRegistry};
use std::sync::Arc;

fn problem() -> DeliveryProblem {
    DeliveryProblem::new("tool-test")
        .with_depot(Depot::new("depot-1", Location::new(0.0, 0.0)))
        .with_vehicle(Vehicle::new("truck-1", "depot-1", 4))
        .with_vehicle(Vehicle::new("truck-2", "depot-1", 4))
        .with_delivery(Delivery::new("a1", Location::new(0.10, 0.10), 2))
        .with_delivery(Delivery::new("a2", Location::new(0.11, 0.11), 2))
        .with_delivery(Delivery::new("b1", Location::new(-0.40, -0.40), 2))
        .with_delivery(Delivery::new("b2", Location::new(-0.41, -0.41), 2))
        .with_matrix(DistanceMatrix::Haversine {
            average_speed_kmh: 40.0,
        })
}

#[test]
fn the_router_brain_does_not_depend_on_any_solver_provider() {
    // Q-Router reaches solvers through the registry. If it ever imports a
    // provider directly, the layering has broken.
    let manifest = include_str!("../../quantumclaw-brains-router/Cargo.toml");
    let dependencies = manifest
        .split("[dev-dependencies]")
        .next()
        .expect("the manifest has a dependency section");

    assert!(
        !dependencies.contains("dwave"),
        "the router crate must not depend on a solver provider: {dependencies}"
    );
}

#[test]
fn every_dwave_backend_is_selectable_by_name() {
    let registry = solver_registry();
    let names = registry.names();

    for expected in [
        "dwave-sa",
        "dwave-sqa",
        "dwave-exact",
        "dwave-hybrid",
        "dwave-qpu",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "{expected} must be selectable, available: {names:?}"
        );
    }
    // Registration must not require Ocean, credentials, or a network call.
    assert_eq!(
        registry.require("dwave-sa").unwrap().kind(),
        SolverKind::Classical
    );
    assert_eq!(
        registry.require("dwave-sqa").unwrap().kind(),
        SolverKind::QuantumInspired
    );
    assert_eq!(
        registry.require("dwave-hybrid").unwrap().kind(),
        SolverKind::QuantumHybrid
    );
    assert_eq!(
        registry.require("dwave-qpu").unwrap().kind(),
        SolverKind::QuantumAnnealing
    );
}

#[test]
fn classical_and_dwave_backends_share_one_selection_mechanism() {
    let registry = solver_registry();

    // Same lookup, same trait object, no provider-specific branch anywhere.
    let classical = registry.require("greedy-classical").unwrap();
    let ocean = registry.require("dwave-sa").unwrap();

    assert_eq!(classical.kind(), ocean.kind());
    assert!(!classical.capabilities().supports_quadratic_models);
    assert!(ocean.capabilities().supports_quadratic_models);
}

#[tokio::test]
async fn a_logistics_task_reaches_the_router_brain_through_the_registry() {
    let selection = brain_registry()
        .select(&AgentTask::new(
            "Optimize tomorrow's deliveries from the São Paulo depot using 25 trucks while respecting capacities and delivery windows",
        ))
        .expect("a logistics task selects a brain");

    assert_eq!(selection.brain.id(), "qrouter");

    let result = selection
        .brain
        .run(
            BrainOperation::Solve,
            serde_json::to_value(QRouterRequest::new(problem())).unwrap(),
            BrainSolveContext::default().with_registry(Arc::new(solver_registry())),
        )
        .await
        .expect("the brain solves through the erased interface");

    assert_eq!(result["kpis"]["deliveries_served"], 4.0);
}

#[tokio::test]
async fn the_qrouter_tool_returns_the_same_plan_as_the_direct_call() {
    let solvers = Arc::new(solver_registry());
    let tools = tool_registry(solvers.clone())
        .await
        .expect("tools register");
    let tool = tools
        .get("qrouter.optimize")
        .await
        .expect("qrouter.optimize is registered");

    let mut call = CoreToolCall::new("qrouter.optimize", "optimize");
    call.input = serde_json::to_value(QRouterRequest::new(problem())).unwrap();
    let through_tool = tool.call(call).await.expect("the tool runs");

    let direct = QRouterBrain::new()
        .solve(
            QRouterRequest::new(problem()),
            BrainSolveContext::default().with_registry(solvers),
        )
        .await
        .expect("the direct call runs");

    assert!(through_tool.success);
    assert_eq!(
        through_tool.output["solution"],
        serde_json::to_value(&direct.solution).unwrap()
    );
}

#[tokio::test]
async fn the_validate_tool_reports_an_impossible_instance_without_optimizing() {
    let tools = tool_registry(Arc::new(solver_registry()))
        .await
        .expect("tools register");
    let tool = tools.get("qrouter.validate").await.expect("tool exists");

    let mut broken = problem();
    broken.deliveries[0].demand = 9_999;
    let mut call = CoreToolCall::new("qrouter.validate", "validate");
    call.input = serde_json::to_value(QRouterRequest::new(broken)).unwrap();

    let result = tool.call(call).await.expect("the tool runs");

    assert!(!result.success);
    assert_eq!(result.output["valid"], false);
}

#[tokio::test]
async fn compare_solvers_explains_the_routing_decision_for_a_problem() {
    let tools = tool_registry(Arc::new(solver_registry()))
        .await
        .expect("tools register");
    let tool = tools
        .get("qrouter.compare_solvers")
        .await
        .expect("tool exists");

    let mut call = CoreToolCall::new("qrouter.compare_solvers", "compare");
    call.input = serde_json::to_value(QRouterRequest::new(problem())).unwrap();
    let result = tool.call(call).await.expect("the tool runs");

    let backends = result.output["backends"]
        .as_array()
        .expect("backends are listed");
    assert!(backends
        .iter()
        .any(|backend| backend["name"] == "dwave-qpu" && backend["requires_credentials"] == true));

    let routing = result.output["routing"]
        .as_array()
        .expect("routing decisions are explained");
    assert!(!routing.is_empty());
    assert!(routing[0]["reason"].as_str().unwrap().len() > 10);
}

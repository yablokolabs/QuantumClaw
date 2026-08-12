//! Behavioral tests for the Q-Router brain.
//!
//! The fixture is deliberately tiny and uses an explicit distance matrix, so
//! every expected number below can be checked by hand.

use quantumclaw_brains::{BrainSolveContext, QuantumBrain};
use quantumclaw_brains_router::benchmark::RouterBenchmark;
use quantumclaw_brains_router::brain::{QRouterBrain, QRouterRequest};
use quantumclaw_brains_router::decomposition::{DecompositionStrategy, DepotPartition};
use quantumclaw_brains_router::models::{
    Delivery, DeliveryProblem, Depot, DistanceMatrix, Location, Route, RouteSolution, TimeWindow,
    Vehicle,
};
use quantumclaw_brains_router::network::Network;
use quantumclaw_brains_router::routing_policy::{
    BenchmarkLedger, LedgerRecord, SolverRoutingPolicy,
};
use quantumclaw_brains_router::{kpis, vrp, ViolationKind};
use quantumclaw_core::{AgentTask, SolverRegistry};
use quantumclaw_optimization::QuboCompiler;
use quantumclaw_providers_dwave::{DWaveBridge, DWaveConfig};
use std::sync::Arc;
use std::time::Duration;

/// A registry with the D-Wave backends, or `None` when this host has no Ocean.
///
/// Set `QUANTUMCLAW_DWAVE_PYTHON` to an interpreter that can import the bridge;
/// `QUANTUMCLAW_DWAVE_REQUIRE=1` turns the skip into a failure.
fn ocean_registry() -> Option<Arc<SolverRegistry>> {
    let python = std::env::var("QUANTUMCLAW_DWAVE_PYTHON")
        .ok()
        .filter(|value| !value.is_empty());
    let Some(python) = python else {
        assert!(
            std::env::var("QUANTUMCLAW_DWAVE_REQUIRE").as_deref() != Ok("1"),
            "QUANTUMCLAW_DWAVE_REQUIRE=1 but QUANTUMCLAW_DWAVE_PYTHON is not set"
        );
        eprintln!("skipping: set QUANTUMCLAW_DWAVE_PYTHON to an interpreter with Ocean installed");
        return None;
    };

    let bridge = Arc::new(DWaveBridge::new(
        DWaveConfig::default()
            .with_python(python)
            .with_timeout(Duration::from_secs(120)),
    ));
    let mut registry = SolverRegistry::new();
    quantumclaw_providers_dwave::register_backends(&mut registry, bridge);
    Some(Arc::new(registry))
}

/// Two tight clusters around one depot:
///
/// ```text
///   d1,d2  (10-12 km out, 3 km apart)   depot   d3,d4 (30-32 km out, 3 km apart)
/// ```
///
/// Every vehicle holds 10 units and every delivery needs 5, so exactly two
/// stops fit per vehicle. The cheapest plan pairs d1 with d2 and d3 with d4,
/// for 25 km + 65 km = 90 km. Splitting the clusters costs 133 km.
fn problem() -> DeliveryProblem {
    let nodes = vec![
        "depot-1".to_string(),
        "d1".to_string(),
        "d2".to_string(),
        "d3".to_string(),
        "d4".to_string(),
    ];
    let distances = vec![
        vec![0.0, 10.0, 12.0, 30.0, 32.0],
        vec![10.0, 0.0, 3.0, 25.0, 27.0],
        vec![12.0, 3.0, 0.0, 22.0, 24.0],
        vec![30.0, 25.0, 22.0, 0.0, 3.0],
        vec![32.0, 27.0, 24.0, 3.0, 0.0],
    ];

    DeliveryProblem::new("sao-paulo-morning")
        .with_depot(Depot::new("depot-1", Location::new(-23.55, -46.63)))
        .with_vehicle(Vehicle::new("truck-1", "depot-1", 10).with_cost_per_km(1.0))
        .with_vehicle(Vehicle::new("truck-2", "depot-1", 10).with_cost_per_km(1.0))
        .with_delivery(Delivery::new("d1", Location::new(-23.5, -46.6), 5))
        .with_delivery(Delivery::new("d2", Location::new(-23.5, -46.59), 5))
        .with_delivery(Delivery::new("d3", Location::new(-23.7, -46.9), 5))
        .with_delivery(Delivery::new("d4", Location::new(-23.71, -46.91), 5))
        .with_matrix(DistanceMatrix::Explicit {
            nodes,
            distances_km: distances,
            durations_min: None,
        })
}

fn request() -> QRouterRequest {
    QRouterRequest::new(problem())
}

/// The set of delivery groups, order-independent, for comparing partitions.
fn clusters(solution: &RouteSolution) -> Vec<Vec<String>> {
    let mut groups: Vec<Vec<String>> = solution
        .routes
        .iter()
        .filter(|route| !route.stops.is_empty())
        .map(|route| {
            let mut stops = route.stops.clone();
            stops.sort();
            stops
        })
        .collect();
    groups.sort();
    groups
}

#[tokio::test]
async fn a_delivery_no_vehicle_can_carry_is_rejected_before_optimizing() {
    let mut problem = problem();
    problem
        .deliveries
        .push(Delivery::new("oversized", Location::new(-23.5, -46.6), 999));

    let report = QRouterBrain::new()
        .validate(&QRouterRequest::new(problem))
        .await
        .expect("validation runs");

    assert!(!report.valid);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.subject == "oversized" && issue.message.contains("exceeds")),
        "expected the oversized delivery to be named: {:?}",
        report.issues
    );
}

#[tokio::test]
async fn a_vehicle_at_an_unknown_depot_is_rejected() {
    let mut problem = problem();
    problem
        .vehicles
        .push(Vehicle::new("ghost-truck", "depot-99", 10));

    let report = QRouterBrain::new()
        .validate(&QRouterRequest::new(problem))
        .await
        .expect("validation runs");

    assert!(!report.valid);
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.message.contains("depot-99")));
}

#[tokio::test]
async fn a_mis_sized_distance_matrix_is_rejected() {
    let mut problem = problem();
    problem.matrix = DistanceMatrix::Explicit {
        nodes: vec!["depot-1".into(), "d1".into()],
        distances_km: vec![vec![0.0, 10.0, 12.0], vec![10.0, 0.0, 3.0]],
        durations_min: None,
    };

    let report = QRouterBrain::new()
        .validate(&QRouterRequest::new(problem))
        .await
        .expect("validation runs");

    assert!(!report.valid);
    assert!(report.issues.iter().any(|issue| issue.subject == "matrix"));
}

#[tokio::test]
async fn optimizing_serves_every_delivery_within_capacity() {
    let result = QRouterBrain::new()
        .solve(request(), BrainSolveContext::default())
        .await
        .expect("the brain solves the instance");

    assert!(result.solution.unassigned.is_empty());
    assert_eq!(result.kpis.deliveries_served, 4);
    assert!(result.feasible, "violations: {:?}", result.violations);
    for route in &result.solution.routes {
        assert!(
            route.stops.len() <= 2,
            "capacity 10 with demand 5 allows two stops, got {:?}",
            route.stops
        );
    }
}

#[tokio::test]
async fn the_classical_path_finds_the_cheaper_of_two_possible_pairings() {
    let result = QRouterBrain::new()
        .solve(request(), BrainSolveContext::default())
        .await
        .expect("the brain solves the instance");

    assert_eq!(
        clusters(&result.solution),
        vec![
            vec!["d1".to_string(), "d2".to_string()],
            vec!["d3".to_string(), "d4".to_string()]
        ]
    );
    assert!(
        (result.kpis.total_distance_km - 90.0).abs() < 1e-6,
        "expected the 90 km plan, got {}",
        result.kpis.total_distance_km
    );
}

#[tokio::test]
async fn capacity_pressure_puts_a_second_vehicle_on_the_road() {
    let mut problem = problem();
    // One vehicle could never carry 20 units.
    problem.vehicles.truncate(1);
    problem.vehicles[0].capacity = 10;
    let single_vehicle_capacity_is_too_small = QRouterBrain::new()
        .validate(&QRouterRequest::new(problem))
        .await
        .expect("validation runs");
    assert!(!single_vehicle_capacity_is_too_small.valid);

    let result = QRouterBrain::new()
        .solve(request(), BrainSolveContext::default())
        .await
        .expect("two vehicles can cover the demand");
    assert_eq!(result.kpis.vehicles_used, 2);
    assert!((result.kpis.fleet_utilization - 1.0).abs() < 1e-9);
    assert!((result.kpis.capacity_utilization - 1.0).abs() < 1e-9);
}

#[test]
fn kpis_match_a_hand_computed_plan() {
    let mut problem = problem();
    // 20 litres per 100 km, 500 g CO2 per km, 1 EUR per km, 60 km/h.
    for vehicle in &mut problem.vehicles {
        vehicle.fuel_l_per_100km = 20.0;
        vehicle.co2_g_per_km = 500.0;
        vehicle.cost_per_km = 1.0;
        vehicle.average_speed_kmh = 60.0;
    }
    problem.cost_model.fuel_price_per_liter = 2.0;
    problem.cost_model.driver_cost_per_hour = 0.0;

    let network = Network::build(&problem).expect("network builds");
    let solution = RouteSolution::new(problem.id.clone())
        .with_route(Route::new("truck-1", "depot-1").with_stops(["d1", "d2"]))
        .with_route(Route::new("truck-2", "depot-1").with_stops(["d3", "d4"]));

    let kpis = kpis::evaluate(&problem, &network, &solution, 0, None);

    // 10 + 3 + 12 = 25 km, and 30 + 3 + 32 = 65 km.
    assert!((kpis.total_distance_km - 90.0).abs() < 1e-9);
    // 90 km at 20 l/100 km.
    assert!((kpis.estimated_fuel_liters - 18.0).abs() < 1e-9);
    // 90 km at 500 g/km.
    assert!((kpis.estimated_co2_kg - 45.0).abs() < 1e-9);
    // 90 km at 1 EUR/km plus 18 litres at 2 EUR.
    assert!(
        (kpis.estimated_operating_cost - 126.0).abs() < 1e-9,
        "got {}",
        kpis.estimated_operating_cost
    );
    assert_eq!(kpis.vehicles_used, 2);
    assert!(kpis.feasible);
}

#[test]
fn a_delivery_served_after_its_window_counts_against_the_sla() {
    let mut problem = problem();
    // 30 km at 40 km/h is 45 minutes, well past a window that closes at 10.
    problem.deliveries[2] = problem.deliveries[2]
        .clone()
        .with_window(TimeWindow::new(0.0, 10.0));

    let network = Network::build(&problem).expect("network builds");
    let solution = RouteSolution::new(problem.id.clone())
        .with_route(Route::new("truck-2", "depot-1").with_stops(["d3", "d4"]));

    let kpis = kpis::evaluate(&problem, &network, &solution, 0, None);

    assert_eq!(kpis.late_deliveries, 1);
    assert!(kpis.sla_violation_minutes > 0.0);
    assert!(!kpis.feasible);
}

#[test]
fn improving_a_route_shortens_a_deliberately_bad_visiting_order() {
    let problem = problem();
    let network = Network::build(&problem).expect("network builds");
    let crossing = vec![
        "d1".to_string(),
        "d3".to_string(),
        "d2".to_string(),
        "d4".to_string(),
    ];

    let improved = vrp::sequence(&network, "depot-1", &crossing);

    let before = network.route_distance_km("depot-1", &crossing);
    let after = network.route_distance_km("depot-1", &improved);
    assert!(
        after < before,
        "sequencing must shorten a crossing route: {before} -> {after}"
    );
}

#[tokio::test]
async fn every_delivery_lands_in_exactly_one_subproblem() {
    let mut problem = problem();
    problem
        .depots
        .push(Depot::new("depot-2", Location::new(-22.9, -43.2)));
    problem
        .vehicles
        .push(Vehicle::new("truck-3", "depot-2", 10));
    let mut nodes = vec!["depot-1".to_string()];
    // A second depot far from everything, so nearest-depot assignment is clear.
    nodes.push("depot-2".into());
    nodes.extend(["d1".to_string(), "d2".into(), "d3".into(), "d4".into()]);
    problem.matrix = DistanceMatrix::Explicit {
        distances_km: vec![
            vec![0.0, 400.0, 10.0, 12.0, 30.0, 32.0],
            vec![400.0, 0.0, 390.0, 392.0, 5.0, 6.0],
            vec![10.0, 390.0, 0.0, 3.0, 25.0, 27.0],
            vec![12.0, 392.0, 3.0, 0.0, 22.0, 24.0],
            vec![30.0, 5.0, 25.0, 22.0, 0.0, 3.0],
            vec![32.0, 6.0, 27.0, 24.0, 3.0, 0.0],
        ],
        nodes,
        durations_min: None,
    };

    let network = Network::build(&problem).expect("network builds");
    let subproblems = DepotPartition
        .decompose(&problem, &network)
        .expect("decomposition runs");

    let mut covered: Vec<String> = subproblems
        .iter()
        .flat_map(|piece| piece.delivery_ids.clone())
        .collect();
    covered.sort();
    assert_eq!(covered, vec!["d1", "d2", "d3", "d4"]);
    assert!(subproblems.len() > 1, "two depots produce two subproblems");
    // d3 and d4 are 5-6 km from depot-2 and 30+ km from depot-1.
    let second = subproblems
        .iter()
        .find(|piece| piece.depot_id == "depot-2")
        .expect("depot-2 gets its own subproblem");
    assert_eq!(second.delivery_ids, vec!["d3", "d4"]);
}

#[tokio::test]
async fn a_large_instance_is_split_until_each_piece_fits_the_variable_budget() {
    let mut problem =
        DeliveryProblem::new("large").with_depot(Depot::new("depot-1", Location::new(0.0, 0.0)));
    for index in 0..40 {
        let angle = f64::from(index) * 0.15;
        problem = problem.with_delivery(Delivery::new(
            format!("d{index}"),
            Location::new(angle.sin(), angle.cos()),
            1,
        ));
    }
    for index in 0..4 {
        problem = problem.with_vehicle(Vehicle::new(format!("truck-{index}"), "depot-1", 20));
    }

    let mut request = QRouterRequest::new(problem);
    request.options.max_variables_per_subproblem = 30;

    let decomposition = QRouterBrain::new()
        .decompose(&request)
        .await
        .expect("decomposition runs");

    assert!(
        decomposition.subproblems.len() > 1,
        "40 deliveries cannot fit one 30-variable model"
    );
    assert!(
        decomposition
            .subproblems
            .iter()
            .all(|piece| piece.size <= 30),
        "every piece must fit the budget: {:?}",
        decomposition
            .subproblems
            .iter()
            .map(|piece| piece.size)
            .collect::<Vec<_>>()
    );
    let mut covered: Vec<String> = decomposition
        .subproblems
        .iter()
        .flat_map(|piece| piece.members.clone())
        .collect();
    covered.sort();
    covered.dedup();
    assert_eq!(covered.len(), 40, "no delivery is lost or duplicated");
}

#[tokio::test]
async fn the_formulated_model_carries_capacity_and_coverage_constraints() {
    let formulations = QRouterBrain::new()
        .formulate(&request())
        .await
        .expect("formulation runs");

    let model = &formulations[0].problem;
    // Four deliveries times two vehicles, plus one use-variable per vehicle.
    assert_eq!(model.variables.len(), 10);
    for delivery in ["d1", "d2", "d3", "d4"] {
        assert!(
            model
                .constraints
                .iter()
                .any(|constraint| constraint.id == format!("serve-{delivery}")),
            "every delivery needs a coverage constraint"
        );
    }
    for vehicle in ["truck-1", "truck-2"] {
        assert!(model
            .constraints
            .iter()
            .any(|constraint| constraint.id == format!("capacity-{vehicle}")));
    }
    assert!(
        !model.quadratic.is_empty(),
        "co-located stops must interact, or there is nothing to search"
    );
}

#[tokio::test]
async fn a_logistics_task_routes_to_this_brain_but_a_coding_task_does_not() {
    let brain = QRouterBrain::new();

    let logistics = brain.can_handle(&AgentTask::new(
        "Optimize tomorrow's deliveries from the São Paulo depot using 25 trucks",
    ));
    let coding = brain.can_handle(&AgentTask::new("Refactor the parser module and add tests"));

    assert!(logistics.score > 0.0, "{}", logistics.reason);
    assert_eq!(coding.score, 0.0, "{}", coding.reason);
}

#[tokio::test]
async fn requesting_a_backend_that_does_not_exist_is_an_error_not_a_silent_fallback() {
    let error = QRouterBrain::new()
        .solve(
            request().with_backend("dwave-qpu"),
            BrainSolveContext::default(),
        )
        .await
        .map(|_| ())
        .expect_err("an explicit backend request must be honoured or fail");

    assert!(error.to_string().contains("dwave-qpu"), "{error}");
}

#[tokio::test]
async fn an_automatic_backend_choice_degrades_to_the_classical_path() {
    // The policy prefers dwave-sa, but no backend is registered.
    let brain = QRouterBrain::new()
        .with_routing_policy(SolverRoutingPolicy::default().with_preferred_backends(["dwave-sa"]));

    let result = brain
        .solve(request(), BrainSolveContext::default())
        .await
        .expect("an unavailable preference must not break the run");

    assert!(result.feasible);
    assert_eq!(result.subproblems[0].backend, "classical-greedy");
    assert!(result.subproblems[0]
        .routing_reason
        .contains("no solver backends"));
}

#[test]
fn the_ledger_recommends_the_backend_that_performed_best_on_similar_problems() {
    let mut ledger = BenchmarkLedger::new();
    for (backend, objective) in [("dwave-sa", 120.0), ("greedy-classical", 180.0)] {
        ledger.record(LedgerRecord {
            class: "vehicle-assignment".into(),
            size_bucket: quantumclaw_brains_router::routing_policy::size_bucket(10),
            backend: backend.into(),
            objective,
            feasible: true,
            runtime_ms: 10,
        });
    }
    // An infeasible run, however cheap, must not recommend a backend.
    ledger.record(LedgerRecord {
        class: "vehicle-assignment".into(),
        size_bucket: quantumclaw_brains_router::routing_policy::size_bucket(10),
        backend: "broken-solver".into(),
        objective: 1.0,
        feasible: false,
        runtime_ms: 1,
    });

    let policy = SolverRoutingPolicy::default().with_ledger(ledger);
    let decision = policy.choose(
        "vehicle-assignment",
        10,
        &[
            "dwave-sa".to_string(),
            "greedy-classical".to_string(),
            "broken-solver".to_string(),
        ],
    );

    assert_eq!(decision.backend.as_deref(), Some("dwave-sa"));
    assert!(
        decision.reason.contains("benchmark evidence"),
        "{}",
        decision.reason
    );
}

#[test]
fn a_ledger_survives_a_round_trip_through_json() {
    let mut ledger = BenchmarkLedger::new();
    ledger.record(LedgerRecord {
        class: "vehicle-assignment".into(),
        size_bucket: 16,
        backend: "dwave-sa".into(),
        objective: 42.0,
        feasible: true,
        runtime_ms: 5,
    });

    let restored =
        BenchmarkLedger::from_json(&ledger.to_json().expect("serializes")).expect("deserializes");

    assert_eq!(
        restored.best_backend("vehicle-assignment", 10, &["dwave-sa".to_string()]),
        Some(("dwave-sa".to_string(), 42.0))
    );
}

#[tokio::test]
async fn benchmarking_reports_the_saving_against_the_customer_baseline() {
    // The customer splits both clusters: 65 + 68 = 133 km against an optimum of 90.
    let baseline = RouteSolution::new("sao-paulo-morning")
        .with_route(Route::new("truck-1", "depot-1").with_stops(["d1", "d3"]))
        .with_route(Route::new("truck-2", "depot-1").with_stops(["d2", "d4"]));
    let request = QRouterRequest::new(problem().with_baseline(baseline));

    let report = RouterBenchmark::new(QRouterBrain::new())
        .run(
            request,
            &["classical".to_string()],
            BrainSolveContext::default(),
        )
        .await
        .expect("the benchmark runs");

    let baseline_entry = report.entry("baseline").expect("baseline is evaluated");
    assert!((baseline_entry.kpis.total_distance_km - 133.0).abs() < 1e-6);

    let optimized = report.entry("classical").expect("classical is evaluated");
    let improvement = optimized
        .improvement
        .as_ref()
        .expect("improvement against the baseline is reported");
    assert!(
        (improvement.distance_km_saved - 43.0).abs() < 1e-6,
        "expected 43 km saved, got {}",
        improvement.distance_km_saved
    );
    assert!(improvement.co2_kg_saved > 0.0);
    assert_eq!(report.winner.as_deref(), Some("classical"));
}

#[tokio::test]
async fn a_candidate_that_fails_never_wins_the_benchmark() {
    let baseline = RouteSolution::new("sao-paulo-morning")
        .with_route(Route::new("truck-1", "depot-1").with_stops(["d1", "d2"]))
        .with_route(Route::new("truck-2", "depot-1").with_stops(["d3", "d4"]));
    let request = QRouterRequest::new(problem().with_baseline(baseline));

    let report = RouterBenchmark::new(QRouterBrain::new())
        .run(
            request,
            &["classical".to_string(), "nonexistent-backend".to_string()],
            BrainSolveContext::default(),
        )
        .await
        .expect("the benchmark runs");

    let failed = report
        .entry("nonexistent-backend")
        .expect("the failing candidate is still reported");
    assert!(failed.error.is_some());
    assert!(!failed.feasible);
    assert_ne!(report.winner.as_deref(), Some("nonexistent-backend"));
}

#[tokio::test]
async fn the_brain_explains_which_backend_produced_the_plan() {
    let brain = QRouterBrain::new();
    let result = brain
        .solve(request(), BrainSolveContext::default())
        .await
        .expect("the brain solves the instance");

    let explanation = brain.explain(&result).await.expect("explanation runs");

    assert!(explanation.summary.contains("4 deliveries"));
    assert!(explanation
        .details
        .iter()
        .any(|detail| detail.contains("Assignment solved by")));
}

#[tokio::test]
async fn an_empty_registry_still_produces_a_usable_plan() {
    let context = BrainSolveContext::default().with_registry(Arc::new(SolverRegistry::new()));

    let result = QRouterBrain::new()
        .solve(request(), context)
        .await
        .expect("the brain does not require any backend");

    assert!(result.feasible);
    assert_eq!(result.kpis.deliveries_served, 4);
}

/// Same two clusters, but sized so the compiled model stays small enough to
/// enumerate: every delivery needs one unit and every vehicle holds two.
fn unit_demand_problem() -> DeliveryProblem {
    let mut problem = problem();
    for delivery in &mut problem.deliveries {
        delivery.demand = 1;
    }
    for vehicle in &mut problem.vehicles {
        vehicle.capacity = 2;
    }
    problem
}

#[tokio::test]
async fn the_compiled_model_scores_the_clustered_pairing_best() {
    // Without interaction terms every pairing costs the same, so this is the
    // test that proves the formulation carries real structure.
    let formulations = QRouterBrain::new()
        .formulate(&QRouterRequest::new(unit_demand_problem()))
        .await
        .expect("formulation runs");
    let compiled = QuboCompiler::default()
        .with_max_exhaustive_variables(24)
        .compile(&formulations[0].problem)
        .expect("the model compiles");

    let best = compiled.brute_force().expect("the model is small enough");

    let mut pairs: Vec<(String, String)> = best
        .selected
        .iter()
        .filter_map(|name| {
            let variable = compiled.problem().variable(name)?;
            Some((
                variable.metadata.get("delivery")?.clone(),
                variable.metadata.get("vehicle")?.clone(),
            ))
        })
        .collect();
    pairs.sort();

    let vehicle_of = |delivery: &str| {
        pairs
            .iter()
            .find(|(id, _)| id == delivery)
            .map(|(_, vehicle)| vehicle.clone())
            .unwrap_or_default()
    };
    assert!(best.feasible, "violations: {:?}", best.violations);
    assert_eq!(
        vehicle_of("d1"),
        vehicle_of("d2"),
        "the two nearby stops belong on the same vehicle"
    );
    assert_eq!(vehicle_of("d3"), vehicle_of("d4"));
    assert_ne!(vehicle_of("d1"), vehicle_of("d3"));
}

#[tokio::test]
async fn exhaustive_ocean_search_produces_the_same_route_plan_as_the_classical_path() {
    let Some(registry) = ocean_registry() else {
        return;
    };

    let result = QRouterBrain::new()
        .solve(
            QRouterRequest::new(unit_demand_problem()).with_backend("dwave-exact"),
            BrainSolveContext::default().with_registry(registry),
        )
        .await
        .expect("the exact backend solves the assignment");

    assert!(result.feasible, "violations: {:?}", result.violations);
    assert_eq!(
        clusters(&result.solution),
        vec![
            vec!["d1".to_string(), "d2".to_string()],
            vec!["d3".to_string(), "d4".to_string()]
        ]
    );
    assert_eq!(result.subproblems[0].backend, "dwave-exact");
    assert!((result.kpis.total_distance_km - 90.0).abs() < 1e-6);
}

#[tokio::test]
async fn simulated_annealing_matches_the_classical_plan_and_reports_its_runtime() {
    let Some(registry) = ocean_registry() else {
        return;
    };

    let result = QRouterBrain::new()
        .solve(
            QRouterRequest::new(unit_demand_problem()).with_backend("dwave-sa"),
            BrainSolveContext::default().with_registry(registry),
        )
        .await
        .expect("simulated annealing solves the assignment");

    assert!(result.feasible, "violations: {:?}", result.violations);
    assert_eq!(
        clusters(&result.solution),
        vec![
            vec!["d1".to_string(), "d2".to_string()],
            vec!["d3".to_string(), "d4".to_string()]
        ]
    );
    assert_eq!(result.subproblems[0].backend, "dwave-sa");
    assert!(
        result.kpis.solver_runtime_ms.is_some(),
        "the provider reports in-solver time separately from wall time"
    );
}

#[tokio::test]
async fn benchmarking_ranks_the_baseline_against_classical_and_ocean_backends() {
    let Some(registry) = ocean_registry() else {
        return;
    };
    let baseline = RouteSolution::new("sao-paulo-morning")
        .with_route(Route::new("truck-1", "depot-1").with_stops(["d1", "d3"]))
        .with_route(Route::new("truck-2", "depot-1").with_stops(["d2", "d4"]));
    let request = QRouterRequest::new(unit_demand_problem().with_baseline(baseline));

    let report = RouterBenchmark::new(QRouterBrain::new())
        .run(
            request,
            &["classical".to_string(), "dwave-sa".to_string()],
            BrainSolveContext::default().with_registry(registry),
        )
        .await
        .expect("the benchmark runs");

    let ocean = report.entry("dwave-sa").expect("the Ocean run is reported");
    assert!(ocean.error.is_none(), "{:?}", ocean.error);
    assert!(ocean.feasible);
    assert!(
        ocean
            .improvement
            .as_ref()
            .expect("improvement over the baseline")
            .distance_km_saved
            > 0.0
    );
    assert_ne!(report.winner.as_deref(), Some("baseline"));
}

/// Capacity exactly equals demand, and greedy placement cannot find the one
/// packing that works: 5 and 3 on the six-unit truck, 4 on the four-unit van.
fn tight_problem() -> DeliveryProblem {
    DeliveryProblem::new("tight-fleet")
        .with_depot(Depot::new("depot-1", Location::new(0.0, 0.0)))
        .with_vehicle(Vehicle::new("truck-1", "depot-1", 6))
        .with_vehicle(Vehicle::new("van-1", "depot-1", 4))
        .with_delivery(Delivery::new("heavy", Location::new(0.10, 0.0), 4))
        .with_delivery(Delivery::new("medium", Location::new(0.11, 0.0), 3))
        .with_delivery(Delivery::new("light", Location::new(0.12, 0.0), 3))
        .with_matrix(DistanceMatrix::Haversine {
            average_speed_kmh: 40.0,
        })
}

#[tokio::test]
async fn a_delivery_the_classical_heuristic_cannot_place_is_reported_not_dropped() {
    let result = QRouterBrain::new()
        .solve(
            QRouterRequest::new(tight_problem()),
            BrainSolveContext::default(),
        )
        .await
        .expect("the brain still returns a plan");

    // Greedy loads the four-unit delivery first and then cannot fit both
    // three-unit ones. What matters is that this is visible, not silent.
    assert_eq!(result.solution.unassigned.len(), 1);
    assert!(!result.feasible);
    assert!(result.violations.iter().any(|violation| {
        violation.kind == ViolationKind::UnassignedDelivery
            && result.solution.unassigned.contains(&violation.subject)
    }));
    assert_eq!(result.kpis.unassigned_deliveries, 1);
}

#[tokio::test]
async fn the_optimization_layer_finds_the_packing_the_heuristic_misses() {
    let Some(registry) = ocean_registry() else {
        return;
    };

    let result = QRouterBrain::new()
        .solve(
            QRouterRequest::new(tight_problem()).with_backend("dwave-exact"),
            BrainSolveContext::default().with_registry(registry),
        )
        .await
        .expect("the exact solver runs");

    assert!(
        result.solution.unassigned.is_empty(),
        "the compiled model has a feasible packing: {:?}",
        result.solution.unassigned
    );
    assert!(result.feasible, "violations: {:?}", result.violations);
    assert_eq!(result.kpis.deliveries_served, 3);
    assert!((result.kpis.capacity_utilization - 1.0).abs() < 1e-9);
}

// --- benchmark reproducibility and ranking -----------------------------------

fn benchmark_problem() -> DeliveryProblem {
    let baseline = RouteSolution::new("sao-paulo-morning")
        .with_route(Route::new("truck-1", "depot-1").with_stops(["d1", "d3"]))
        .with_route(Route::new("truck-2", "depot-1").with_stops(["d2", "d4"]));
    unit_demand_problem().with_baseline(baseline)
}

#[tokio::test]
async fn the_same_seed_reproduces_the_same_benchmark() {
    let Some(registry) = ocean_registry() else {
        return;
    };
    let candidates = ["classical".to_string(), "dwave-sa".to_string()];

    let run = || async {
        RouterBenchmark::new(QRouterBrain::new())
            .with_repetitions(3)
            .with_seed(4242)
            .run(
                QRouterRequest::new(benchmark_problem()),
                &candidates,
                BrainSolveContext::default().with_registry(registry.clone()),
            )
            .await
            .expect("the benchmark runs")
    };

    let first = run().await;
    let second = run().await;

    // A stochastic sampler must not produce a different verdict on a rerun.
    assert_eq!(first.winner, second.winner);
    for (left, right) in first.entries.iter().zip(second.entries.iter()) {
        let (Some(left_stats), Some(right_stats)) = (&left.stats, &right.stats) else {
            continue;
        };
        assert_eq!(
            left_stats.objective_median, right_stats.objective_median,
            "{} moved between identical runs",
            left.label
        );
        assert_eq!(left_stats.seeds, right_stats.seeds);
    }
}

#[tokio::test]
async fn every_candidate_reports_the_spread_across_its_runs() {
    let report = RouterBenchmark::new(QRouterBrain::new())
        .with_repetitions(4)
        .with_seed(7)
        .run(
            QRouterRequest::new(benchmark_problem()),
            &["classical".to_string()],
            BrainSolveContext::default(),
        )
        .await
        .expect("the benchmark runs");

    let stats = report
        .entry("classical")
        .and_then(|entry| entry.stats.as_ref())
        .expect("repeated runs are summarised");

    assert_eq!(stats.runs, 4);
    assert_eq!(stats.seeds, vec![7, 8, 9, 10]);
    assert!(stats.objective_best <= stats.objective_median);
    assert!(stats.objective_median <= stats.objective_worst);
    // The classical path is deterministic, so its runs must not vary at all.
    assert!(
        stats.objective_stddev < 1e-9,
        "a deterministic solver reported a spread of {}",
        stats.objective_stddev
    );
}

#[tokio::test]
async fn a_candidate_that_is_only_sometimes_feasible_cannot_win() {
    // The baseline is a fixed infeasible plan, so it must never be the winner
    // however cheap it looks.
    let mut problem = benchmark_problem();
    problem.vehicles[0].capacity = 1;
    problem.vehicles[1].capacity = 1;

    let report = RouterBenchmark::new(QRouterBrain::new())
        .with_repetitions(2)
        .run(
            QRouterRequest::new(problem),
            &["classical".to_string()],
            BrainSolveContext::default(),
        )
        .await
        .expect("the benchmark runs");

    if let Some(entry) = report.entry("classical") {
        if let Some(stats) = &entry.stats {
            if !stats.always_feasible() {
                assert_ne!(report.winner.as_deref(), Some("classical"));
            }
        }
    }
    assert_ne!(report.winner.as_deref(), Some("baseline"));
}

#[tokio::test]
async fn a_single_repetition_is_flagged_as_not_a_measurement() {
    let report = RouterBenchmark::new(QRouterBrain::new())
        .with_repetitions(1)
        .run(
            QRouterRequest::new(benchmark_problem()),
            &["classical".to_string()],
            BrainSolveContext::default(),
        )
        .await
        .expect("the benchmark runs");

    assert!(
        report.notes.iter().any(|note| note.contains("luck")),
        "a one-shot benchmark must say so: {:?}",
        report.notes
    );
}

#[tokio::test]
async fn different_seeds_reach_the_sampler() {
    let Some(registry) = ocean_registry() else {
        return;
    };

    // Same problem, different seeds: the plumbing is proven by the runs being
    // reproducible per seed, not by them differing (a tiny instance may well
    // give the same optimum every time).
    let solve = |seed: u64| {
        let registry = registry.clone();
        async move {
            QRouterBrain::new()
                .solve(
                    QRouterRequest::new(unit_demand_problem())
                        .with_backend("dwave-sa")
                        .with_sampler_seed(seed),
                    BrainSolveContext::default().with_registry(registry),
                )
                .await
                .expect("the brain solves")
        }
    };

    let first = solve(11).await;
    let repeat = solve(11).await;

    assert_eq!(
        first.solution.routes, repeat.solution.routes,
        "the same seed must produce the same plan"
    );
    assert!(first.feasible);
}

/// A backend that records nothing and solves nothing, registered only so the
/// routing policy has something to prefer.
struct NeverCalledBackend;

#[async_trait::async_trait]
impl quantumclaw_core::SolverBackend for NeverCalledBackend {
    fn name(&self) -> &'static str {
        "dwave-sa"
    }

    fn kind(&self) -> quantumclaw_core::SolverKind {
        quantumclaw_core::SolverKind::Classical
    }

    async fn solve(
        &self,
        _problem: quantumclaw_ir::DecisionProblem,
        _context: quantumclaw_core::SolverContext,
    ) -> quantumclaw_core::Result<quantumclaw_core::SolverOutput> {
        panic!("the classical path must not reach a sampler");
    }
}

#[tokio::test]
async fn asking_for_the_classical_path_does_not_reach_a_sampler() {
    // Regression: `backend: None` means "let the policy choose", and the
    // policy prefers dwave-sa. A benchmark row labelled classical was
    // therefore running the sampler, making every classical-vs-sampler
    // comparison a comparison of the sampler with itself.
    let mut registry = SolverRegistry::new();
    registry.register(Arc::new(NeverCalledBackend));

    let result = QRouterBrain::new()
        .solve(
            request().with_backend(quantumclaw_brains_router::BACKEND_CLASSICAL),
            BrainSolveContext::default().with_registry(Arc::new(registry)),
        )
        .await
        .expect("the classical path solves without any backend");

    assert_eq!(result.subproblems[0].backend, "classical-greedy");
    assert!(result.subproblems[0]
        .routing_reason
        .contains("classical path explicitly"));
}

#[tokio::test]
async fn the_benchmark_classical_row_is_actually_classical() {
    let mut registry = SolverRegistry::new();
    registry.register(Arc::new(NeverCalledBackend));

    let report = RouterBenchmark::new(QRouterBrain::new())
        .with_repetitions(2)
        .run(
            QRouterRequest::new(benchmark_problem()),
            &[quantumclaw_brains_router::BACKEND_CLASSICAL.to_string()],
            BrainSolveContext::default().with_registry(Arc::new(registry)),
        )
        .await
        .expect("the benchmark runs");

    let entry = report.entry("classical").expect("classical was evaluated");
    assert_eq!(entry.backend, "classical-greedy");
    // Deterministic, so repeated runs must agree exactly.
    let stats = entry.stats.as_ref().expect("runs are summarised");
    assert!(stats.objective_stddev < 1e-9);
}

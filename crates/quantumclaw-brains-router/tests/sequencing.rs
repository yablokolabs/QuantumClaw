//! Behavioral tests for QUBO-based route sequencing.
//!
//! The fixture is a square with a known optimal tour, so every expectation
//! below is checkable by hand.

use quantumclaw_brains::{BrainSolveContext, QuantumBrain};
use quantumclaw_brains_router::brain::{QRouterBrain, QRouterRequest};
use quantumclaw_brains_router::models::{
    Delivery, DeliveryProblem, Depot, DistanceMatrix, Location, Vehicle,
};
use quantumclaw_brains_router::network::Network;
use quantumclaw_brains_router::sequencing::{
    decode_sequence, tsp_problem, SequencingChoice, SequencingPolicy,
};
use quantumclaw_core::SolverRegistry;
use quantumclaw_optimization::QuboCompiler;
use quantumclaw_providers_dwave::{DWaveBridge, DWaveConfig};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// A depot at one corner and four stops around a square.
///
/// ```text
///   depot --- a --- b
///               |     |
///             d --- c
/// ```
///
/// The perimeter tour `depot -> a -> b -> c -> d -> depot` costs
/// 1 + 1 + 1 + 1 + 1 = 5.0 km. Any tour crossing the middle pays a diagonal
/// (1.41 or 2.24) and costs more, so the perimeter is the unique optimum up
/// to direction.
fn square_problem() -> DeliveryProblem {
    let nodes = vec![
        "depot".to_string(),
        "a".to_string(),
        "b".to_string(),
        "c".to_string(),
        "d".to_string(),
    ];
    // depot is 1 from a and d, 2 from b, and 2.24 from c (diagonal).
    let distances = vec![
        //        depot    a     b     c     d
        vec![0.00, 1.00, 2.00, 2.24, 1.00],
        vec![1.00, 0.00, 1.00, 1.41, 1.00],
        vec![2.00, 1.00, 0.00, 1.00, 1.41],
        vec![2.24, 1.41, 1.00, 0.00, 1.00],
        vec![1.00, 1.00, 1.41, 1.00, 0.00],
    ];

    let mut problem = DeliveryProblem::new("square")
        .with_depot(Depot::new("depot", Location::new(0.0, 0.0)))
        .with_vehicle(Vehicle::new("truck-1", "depot", 10).with_cost_per_km(1.0))
        .with_matrix(DistanceMatrix::Explicit {
            nodes,
            distances_km: distances,
            durations_min: None,
        });
    for id in ["a", "b", "c", "d"] {
        problem = problem.with_delivery(Delivery::new(id, Location::new(0.0, 0.0), 1));
    }
    problem
}

fn network() -> Network {
    Network::build(&square_problem()).expect("network builds")
}

fn stops() -> Vec<String> {
    ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect()
}

/// Fails when the report shows the sampler never actually ran.
///
/// The sequencing pass deliberately swallows backend errors into `reason` so a
/// broken sampler cannot break a delivery plan. That is right for production
/// and wrong for a test: without this check, these tests pass on a host where
/// the bridge is not installed at all.
fn assert_sampler_ran(report: &quantumclaw_brains_router::SequencingReport) {
    assert_ne!(
        report.backend, "classical",
        "no sampler was even selected: {}",
        report.reason
    );
    for broken in [
        "not installed",
        "failed",
        "not registered",
        "no sampling backend",
    ] {
        assert!(
            !report.reason.contains(broken),
            "the sampler did not run ({broken}): {}",
            report.reason
        );
    }
}

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

#[test]
fn the_tsp_model_uses_one_variable_per_stop_and_position() {
    let model = tsp_problem(&network(), "depot", &stops()).expect("model builds");

    // Four stops over four positions.
    assert_eq!(model.variables.len(), 16);
    // Each stop visited once, and each position filled once.
    assert_eq!(model.constraints.len(), 8);
}

#[test]
fn exhaustive_search_over_the_tsp_model_finds_the_perimeter_tour() {
    let network = network();
    let compiled = QuboCompiler::default()
        .with_max_exhaustive_variables(16)
        .compile(&tsp_problem(&network, "depot", &stops()).expect("model builds"))
        .expect("model compiles");

    let best = compiled.brute_force().expect("16 variables is enumerable");
    let sequence = decode_sequence(&best, compiled.problem()).expect("a valid tour decodes");

    assert!(best.feasible, "violations: {:?}", best.violations);
    // Either direction round the square is optimal; both cost 5.0 km.
    assert!(
        sequence == ["a", "b", "c", "d"] || sequence == ["d", "c", "b", "a"],
        "expected the perimeter tour, got {sequence:?}"
    );
    assert!(
        (network.route_distance_km("depot", &sequence) - 5.0).abs() < 1e-6,
        "expected the 5.0 km perimeter tour, got {}",
        network.route_distance_km("depot", &sequence)
    );
}

#[test]
fn a_sample_that_puts_two_stops_in_one_position_is_rejected() {
    let network = network();
    let model = tsp_problem(&network, "depot", &stops()).expect("model builds");
    let compiled = QuboCompiler::default().compile(&model).expect("compiles");

    // Both `a` and `b` claim position 0, and positions 2 and 3 are empty.
    let mut sample = BTreeMap::new();
    for variable in &model.variables {
        sample.insert(variable.name.clone(), 0);
    }
    sample.insert("visit::a::0".into(), 1);
    sample.insert("visit::b::0".into(), 1);
    sample.insert("visit::c::1".into(), 1);

    let decoded = compiled.decode(&sample);

    assert!(
        decode_sequence(&decoded, &model).is_none(),
        "a broken tour must be rejected, not turned into a route"
    );
}

#[test]
fn a_tour_missing_a_stop_is_rejected() {
    let network = network();
    let model = tsp_problem(&network, "depot", &stops()).expect("model builds");
    let compiled = QuboCompiler::default().compile(&model).expect("compiles");

    let mut sample = BTreeMap::new();
    for variable in &model.variables {
        sample.insert(variable.name.clone(), 0);
    }
    // Three of four stops placed.
    sample.insert("visit::a::0".into(), 1);
    sample.insert("visit::b::1".into(), 1);
    sample.insert("visit::c::2".into(), 1);

    assert!(decode_sequence(&compiled.decode(&sample), &model).is_none());
}

#[test]
fn a_route_larger_than_the_guard_is_not_offered_to_a_sampler() {
    let policy = SequencingPolicy::default();

    assert!(policy.accepts(policy.max_stops));
    assert!(
        !policy.accepts(policy.max_stops + 1),
        "the guard must refuse routes above its threshold: n^2 variables grows fast"
    );
}

#[test]
fn the_guard_default_keeps_the_model_small_enough_to_sample() {
    let policy = SequencingPolicy::default();

    // n stops produce n^2 binary variables plus penalty terms.
    assert!(
        policy.max_stops * policy.max_stops <= 100,
        "default guard would allow a {}-variable model",
        policy.max_stops * policy.max_stops
    );
}

#[tokio::test]
async fn sequencing_is_off_unless_it_is_asked_for() {
    let result = QRouterBrain::new()
        .solve(
            QRouterRequest::new(square_problem()),
            BrainSolveContext::default(),
        )
        .await
        .expect("the brain solves");

    assert!(
        result.sequencing.is_empty(),
        "QUBO sequencing must be opt-in, got {:?}",
        result.sequencing
    );
}

#[tokio::test]
async fn qubo_sequencing_never_produces_a_worse_route_than_the_classical_one() {
    let Some(registry) = ocean_registry() else {
        return;
    };

    let mut request = QRouterRequest::new(square_problem());
    request.options.sequencing = SequencingPolicy::default().enabled();

    let result = QRouterBrain::new()
        .solve(
            request,
            BrainSolveContext::default().with_registry(registry),
        )
        .await
        .expect("the brain solves");

    let report = result
        .sequencing
        .first()
        .expect("the route was considered for QUBO sequencing");
    assert_sampler_ran(report);

    // Whatever the sampler returned, the shipped route is never longer than
    // what the classical heuristic already had.
    let shipped = result.kpis.total_distance_km;
    assert!(
        shipped <= report.classical_distance_km + 1e-9,
        "shipped {shipped} km against a classical {} km",
        report.classical_distance_km
    );
    assert!(result.feasible);
}

#[tokio::test]
async fn a_qubo_sequenced_route_still_visits_every_stop_exactly_once() {
    let Some(registry) = ocean_registry() else {
        return;
    };

    let mut request = QRouterRequest::new(square_problem());
    request.options.sequencing = SequencingPolicy::default().enabled();

    let result = QRouterBrain::new()
        .solve(
            request,
            BrainSolveContext::default().with_registry(registry),
        )
        .await
        .expect("the brain solves");

    let mut visited: Vec<String> = result
        .solution
        .routes
        .iter()
        .flat_map(|route| route.stops.clone())
        .collect();
    visited.sort();

    assert_eq!(visited, vec!["a", "b", "c", "d"]);
    assert!(result.solution.unassigned.is_empty());
}

#[tokio::test]
async fn the_sequencing_report_says_which_method_won() {
    let Some(registry) = ocean_registry() else {
        return;
    };

    let mut request = QRouterRequest::new(square_problem());
    request.options.sequencing = SequencingPolicy::default().enabled();

    let result = QRouterBrain::new()
        .solve(
            request,
            BrainSolveContext::default().with_registry(registry),
        )
        .await
        .expect("the brain solves");

    let report = &result.sequencing[0];
    assert_sampler_ran(report);
    assert_eq!(report.stops, 4);
    assert_eq!(report.variables, 16);
    match report.chosen {
        SequencingChoice::Qubo => {
            let qubo = report.qubo_distance_km.expect("a QUBO route was decoded");
            assert!(qubo <= report.classical_distance_km + 1e-9);
        }
        SequencingChoice::Classical => {
            assert!(!report.reason.is_empty(), "a rejection must say why");
        }
    }
}

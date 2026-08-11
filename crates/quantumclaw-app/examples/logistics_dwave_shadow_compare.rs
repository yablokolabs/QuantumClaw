//! End-to-end logistics example.
//!
//! ```text
//! DeliveryProblem -> Q-Router -> OptimizationProblem -> QUBO -> dwave-sa
//!                 -> decoded routes -> logistics KPIs -> benchmark
//! ```
//!
//! Run it with the classical path only:
//!
//! ```sh
//! cargo run -p quantumclaw-app --example logistics_dwave_shadow_compare
//! ```
//!
//! Or against Ocean's classical simulated annealing sampler:
//!
//! ```sh
//! QUANTUMCLAW_DWAVE_PYTHON=.venv-dwave/bin/python \
//!   cargo run -p quantumclaw-app --example logistics_dwave_shadow_compare
//! ```

use quantumclaw_app::solver_registry;
use quantumclaw_brains::{BrainSolveContext, QuantumBrain};
use quantumclaw_brains_router::benchmark::RouterBenchmark;
use quantumclaw_brains_router::brain::{QRouterBrain, QRouterRequest};
use quantumclaw_brains_router::models::{
    Delivery, DeliveryProblem, Depot, DistanceMatrix, Location, Route, RouteSolution, Vehicle,
};
use quantumclaw_core::Result;
use std::sync::Arc;

/// Six deliveries in two clusters, three vehicles that each hold two stops.
///
/// The distances are explicit so the example prints the same numbers on every
/// machine.
fn delivery_problem() -> DeliveryProblem {
    let nodes = vec![
        "depot-sp".to_string(),
        "west-1".to_string(),
        "west-2".to_string(),
        "west-3".to_string(),
        "east-1".to_string(),
        "east-2".to_string(),
        "east-3".to_string(),
    ];
    // Two tight clusters: west sits 8-11 km out, east sits 26-30 km out.
    let distances_km = vec![
        vec![0.0, 8.0, 9.0, 11.0, 26.0, 28.0, 30.0],
        vec![8.0, 0.0, 2.0, 3.0, 30.0, 32.0, 34.0],
        vec![9.0, 2.0, 0.0, 2.0, 31.0, 33.0, 35.0],
        vec![11.0, 3.0, 2.0, 0.0, 32.0, 34.0, 36.0],
        vec![26.0, 30.0, 31.0, 32.0, 0.0, 3.0, 5.0],
        vec![28.0, 32.0, 33.0, 34.0, 3.0, 0.0, 2.0],
        vec![30.0, 34.0, 35.0, 36.0, 5.0, 2.0, 0.0],
    ];

    let mut problem = DeliveryProblem::new("sao-paulo-morning")
        .with_depot(Depot::new("depot-sp", Location::new(-23.55, -46.63)))
        .with_matrix(DistanceMatrix::Explicit {
            nodes,
            distances_km,
            durations_min: None,
        });

    for index in 1..=3 {
        problem = problem.with_vehicle(
            Vehicle::new(format!("truck-{index}"), "depot-sp", 2)
                .with_cost_per_km(1.1)
                .with_fuel_l_per_100km(26.0)
                .with_co2_g_per_km(690.0),
        );
    }
    for id in ["west-1", "west-2", "west-3", "east-1", "east-2", "east-3"] {
        problem = problem.with_delivery(Delivery::new(id, Location::new(-23.5, -46.6), 1));
    }

    // What the dispatcher does today: one stop from each cluster per truck.
    problem.with_baseline(
        RouteSolution::new("sao-paulo-morning")
            .with_route(Route::new("truck-1", "depot-sp").with_stops(["west-1", "east-1"]))
            .with_route(Route::new("truck-2", "depot-sp").with_stops(["west-2", "east-2"]))
            .with_route(Route::new("truck-3", "depot-sp").with_stops(["west-3", "east-3"])),
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let registry = Arc::new(solver_registry());
    let brain = QRouterBrain::new();
    let context = BrainSolveContext::default().with_registry(registry.clone());
    let request = QRouterRequest::new(delivery_problem());

    println!("== formulation ==");
    for formulation in brain.formulate(&request).await? {
        println!(
            "{}: {} binary variables, {} constraints, {} interactions",
            formulation.id,
            formulation.problem.variables.len(),
            formulation.problem.constraints.len(),
            formulation.problem.quadratic.len()
        );
    }

    // Which backends can actually run here? dwave-sa needs the Ocean bridge.
    // dwave-exact is deliberately absent: this model has more variables than
    // its safety threshold allows, because exhaustive search is exponential.
    let mut candidates = vec!["classical".to_string()];
    if std::env::var("QUANTUMCLAW_DWAVE_PYTHON").is_ok() {
        candidates.push("dwave-sa".to_string());
    } else {
        println!(
            "\nset QUANTUMCLAW_DWAVE_PYTHON to an interpreter with Ocean installed to add the \
             dwave-sa and dwave-exact lanes"
        );
    }

    println!("\n== benchmark ==");
    let report = RouterBenchmark::new(brain.clone())
        .run(request.clone(), &candidates, context.clone())
        .await?;

    println!(
        "{:<14} {:>10} {:>10} {:>9} {:>8} {:>10}",
        "candidate", "distance", "cost", "vehicles", "co2 kg", "runtime ms"
    );
    for entry in &report.entries {
        if let Some(error) = &entry.error {
            println!("{:<14} failed: {error}", entry.label);
            continue;
        }
        println!(
            "{:<14} {:>10.1} {:>10.2} {:>9} {:>8.1} {:>10}",
            entry.label,
            entry.kpis.total_distance_km,
            entry.kpis.objective_value,
            entry.kpis.vehicles_used,
            entry.kpis.estimated_co2_kg,
            entry.kpis.optimization_runtime_ms,
        );
        if let Some(improvement) = &entry.improvement {
            println!(
                "{:<14} saves {:.1} km ({:.0}%), {:.1} kg CO2 against the customer baseline",
                "",
                improvement.distance_km_saved,
                improvement.distance_improvement.unwrap_or(0.0) * 100.0,
                improvement.co2_kg_saved,
            );
        }
    }
    println!("winner: {}", report.winner.as_deref().unwrap_or("none"));

    println!("\n== plan ==");
    let result = brain.solve(request, context).await?;
    for route in &result.solution.routes {
        println!("{}: {}", route.vehicle_id, route.stops.join(" -> "));
    }
    for detail in brain.explain(&result).await?.details {
        println!("- {detail}");
    }

    Ok(())
}

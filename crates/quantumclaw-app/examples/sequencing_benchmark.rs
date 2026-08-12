//! Measures QUBO route sequencing against classical local search.
//!
//! Both are compared against the **exact optimum**, found by enumerating every
//! permutation. That is the informative comparison: knowing which heuristic
//! wins says little if both already find the optimum, and this is the only way
//! to tell.
//!
//! ```sh
//! QUANTUMCLAW_DWAVE_PYTHON=.venv-dwave/bin/python \
//!   cargo run -p quantumclaw-app --release --example sequencing_benchmark
//! ```
//!
//! Instances are generated from a fixed seed, so the numbers are reproducible.

use quantumclaw_brains_router::models::{
    Delivery, DeliveryProblem, Depot, DistanceMatrix, Location, Vehicle,
};
use quantumclaw_brains_router::network::Network;
use quantumclaw_brains_router::sequencing::{decode_sequence, tsp_problem};
use quantumclaw_brains_router::vrp;
use quantumclaw_core::{AgentTask, Result, SolverBackend, SolverContext};
use quantumclaw_ir::DecisionProblem;
use quantumclaw_providers_dwave::{
    DWaveBridge, DWaveConfig, DWaveSimulatedAnnealingBackend, SimulatedAnnealingParams,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Deterministic generator, so a run can be reproduced exactly.
struct Rng(u64);

impl Rng {
    fn next_f64(&mut self) -> f64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A depot plus `stops` deliveries scattered over a 20 km square.
fn instance(seed: u64, stops: usize) -> DeliveryProblem {
    let mut rng = Rng(seed | 1);
    let mut problem = DeliveryProblem::new(format!("bench-{seed}-{stops}"))
        .with_depot(Depot::new("depot", Location::new(0.0, 0.0)))
        .with_vehicle(Vehicle::new("truck", "depot", stops as u32))
        .with_matrix(DistanceMatrix::Haversine {
            average_speed_kmh: 40.0,
        });
    for index in 0..stops {
        // Roughly +/- 0.1 degrees, about 11 km.
        let lat = (rng.next_f64() - 0.5) * 0.2;
        let lon = (rng.next_f64() - 0.5) * 0.2;
        problem = problem.with_delivery(Delivery::new(
            format!("s{index}"),
            Location::new(lat, lon),
            1,
        ));
    }
    problem
}

/// Shortest tour over every permutation. Exact, and the yardstick.
fn optimal_tour(network: &Network, depot: &str, stops: &[String]) -> f64 {
    let mut order: Vec<usize> = (0..stops.len()).collect();
    let mut best = f64::INFINITY;

    // Heap's algorithm, iterative.
    let mut counters = vec![0usize; stops.len()];
    let evaluate = |order: &[usize]| -> f64 {
        let tour: Vec<String> = order.iter().map(|index| stops[*index].clone()).collect();
        network.route_distance_km(depot, &tour)
    };
    best = best.min(evaluate(&order));

    let mut index = 0;
    while index < order.len() {
        if counters[index] < index {
            if index % 2 == 0 {
                order.swap(0, index);
            } else {
                order.swap(counters[index], index);
            }
            best = best.min(evaluate(&order));
            counters[index] += 1;
            index = 0;
        } else {
            counters[index] = 0;
            index += 1;
        }
    }

    best
}

struct Tally {
    runs: usize,
    classical_optimal: usize,
    qubo_optimal: usize,
    qubo_invalid: usize,
    qubo_strictly_better: usize,
    classical_strictly_better: usize,
    classical_excess: f64,
    qubo_excess: f64,
    classical_ms: f64,
    qubo_ms: f64,
}

impl Tally {
    fn new() -> Self {
        Self {
            runs: 0,
            classical_optimal: 0,
            qubo_optimal: 0,
            qubo_invalid: 0,
            qubo_strictly_better: 0,
            classical_strictly_better: 0,
            classical_excess: 0.0,
            qubo_excess: 0.0,
            classical_ms: 0.0,
            qubo_ms: 0.0,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let python = std::env::var("QUANTUMCLAW_DWAVE_PYTHON").unwrap_or_else(|_| "python3".into());
    let bridge = Arc::new(DWaveBridge::new(
        DWaveConfig::default()
            .with_python(&python)
            .with_timeout(Duration::from_secs(60)),
    ));
    let reads: u32 = std::env::var("BENCH_NUM_READS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(100);
    let backend = DWaveSimulatedAnnealingBackend::new(bridge).with_params(
        SimulatedAnnealingParams::default()
            .with_num_reads(reads)
            .with_seed(7),
    );
    let instances: usize = std::env::var("BENCH_INSTANCES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20);

    println!("QUBO sequencing vs classical local search, both against the exact optimum");
    println!("num_reads={reads}, {instances} instances per size, seeded\n");
    println!(
        "{:>5} {:>6} {:>11} {:>11} {:>9} {:>10} {:>10} {:>9} {:>9}",
        "stops",
        "vars",
        "classical",
        "qubo",
        "invalid",
        "cls excess",
        "qbo excess",
        "cls ms",
        "qbo ms"
    );
    println!("{}", "-".repeat(90));

    for stops in 3..=8usize {
        let mut tally = Tally::new();

        for run in 0..instances {
            let problem = instance((run as u64 + 1) * 7919 + stops as u64, stops);
            let network = Network::build(&problem)?;
            let ids: Vec<String> = problem
                .deliveries
                .iter()
                .map(|delivery| delivery.id.clone())
                .collect();

            let optimum = optimal_tour(&network, "depot", &ids);

            let started = Instant::now();
            let classical = vrp::sequence(&network, "depot", &ids);
            tally.classical_ms += started.elapsed().as_secs_f64() * 1000.0;
            let classical_distance = network.route_distance_km("depot", &classical);

            let model = tsp_problem(&network, "depot", &ids)?;
            let started = Instant::now();
            let output = backend
                .solve(
                    DecisionProblem::new("bench").with_optimization(model.clone()),
                    SolverContext::from_task(&AgentTask::new("sequence")),
                )
                .await?;
            tally.qubo_ms += started.elapsed().as_secs_f64() * 1000.0;

            let qubo_distance = output
                .solution
                .as_ref()
                .and_then(|solution| decode_sequence(solution, &model))
                .map(|tour| network.route_distance_km("depot", &tour));

            tally.runs += 1;
            if (classical_distance - optimum).abs() < 1e-6 {
                tally.classical_optimal += 1;
            }
            tally.classical_excess += (classical_distance - optimum) / optimum;

            match qubo_distance {
                Some(distance) => {
                    if (distance - optimum).abs() < 1e-6 {
                        tally.qubo_optimal += 1;
                    }
                    tally.qubo_excess += (distance - optimum) / optimum;
                    if distance + 1e-9 < classical_distance {
                        tally.qubo_strictly_better += 1;
                    } else if classical_distance + 1e-9 < distance {
                        tally.classical_strictly_better += 1;
                    }
                }
                None => tally.qubo_invalid += 1,
            }
        }

        let valid = (tally.runs - tally.qubo_invalid).max(1) as f64;
        println!(
            "{:>5} {:>6} {:>7}/{:<3} {:>7}/{:<3} {:>9} {:>9.2}% {:>9.2}% {:>9.2} {:>9.1}",
            stops,
            stops * stops,
            tally.classical_optimal,
            tally.runs,
            tally.qubo_optimal,
            tally.runs,
            tally.qubo_invalid,
            tally.classical_excess / tally.runs as f64 * 100.0,
            tally.qubo_excess / valid * 100.0,
            tally.classical_ms / tally.runs as f64,
            tally.qubo_ms / tally.runs as f64,
        );

        if tally.qubo_strictly_better > 0 || tally.classical_strictly_better > 0 {
            println!(
                "      head to head: QUBO shorter {} time(s), classical shorter {} time(s)",
                tally.qubo_strictly_better, tally.classical_strictly_better
            );
        }
    }

    println!("\n'excess' is the mean gap above the exact optimum. 0.00% means the");
    println!("method found the optimal tour on every instance it produced one for.");
    Ok(())
}

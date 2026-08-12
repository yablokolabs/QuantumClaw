//! Route sequencing as a binary optimization problem.
//!
//! Ordering the stops on one vehicle is a travelling-salesman problem. It is
//! expressed here with the standard position encoding: one binary per
//! (stop, position) pair, constrained so every stop takes exactly one position
//! and every position holds exactly one stop.
//!
//! ```text
//!   x[i][p] = 1  <=>  stop i is visited p-th
//! ```
//!
//! **This is opt-in and deliberately guarded.** The encoding costs `n^2`
//! variables, and for the cluster sizes Q-Router produces, nearest-neighbour
//! plus 2-opt already finds the optimum almost every time at a fraction of the
//! cost. The QUBO lane exists so that claim can be *measured* on real hardware
//! rather than asserted — not because it is expected to win.
//!
//! Whatever a sampler returns, [`SequencingChoice`] records which route was
//! actually shipped, and the caller keeps the shorter of the two. A sampler
//! can never make a route worse than the classical heuristic already had.

use crate::network::Network;
use quantumclaw_core::{QuantumClawError, Result};
use quantumclaw_ir::optimization::{
    BinaryVariable, OptimizationConstraint, OptimizationProblem, OptimizationSolution,
};
use serde::{Deserialize, Serialize};

/// Prefix of a (stop, position) variable.
pub const VISIT_PREFIX: &str = "visit";

/// Name of the variable placing `stop` at `position`.
pub fn visit_variable(stop: &str, position: usize) -> String {
    format!("{VISIT_PREFIX}::{stop}::{position}")
}

/// When a route may be sequenced by a sampler rather than by local search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequencingPolicy {
    /// Off by default: classical sequencing is cheaper and usually optimal at
    /// these sizes.
    pub enabled: bool,
    /// Largest route the QUBO lane will attempt. The model has `max_stops^2`
    /// binary variables before penalties, so this guard is not decorative.
    pub max_stops: usize,
    /// Backend to sequence with. Falls back to the caller's requested backend.
    pub backend: Option<String>,
}

impl Default for SequencingPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_stops: 8,
            backend: None,
        }
    }
}

impl SequencingPolicy {
    pub fn enabled(mut self) -> Self {
        self.enabled = true;
        self
    }

    pub fn with_max_stops(mut self, max_stops: usize) -> Self {
        self.max_stops = max_stops;
        self
    }

    pub fn with_backend(mut self, backend: impl Into<String>) -> Self {
        self.backend = Some(backend.into());
        self
    }

    /// Whether a route of this size may be offered to a sampler.
    ///
    /// Routes of two or fewer stops have nothing to optimize: every ordering
    /// of a closed tour is the same length.
    pub fn accepts(&self, stops: usize) -> bool {
        stops >= 3 && stops <= self.max_stops
    }
}

/// Which sequencing method produced the route that was shipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequencingChoice {
    Classical,
    Qubo,
}

/// What happened when a route was considered for QUBO sequencing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequencingReport {
    pub vehicle_id: String,
    pub stops: usize,
    pub variables: usize,
    /// Backend that sampled the model, or `classical` when none did.
    pub backend: String,
    pub classical_distance_km: f64,
    /// Absent when the sampler produced nothing usable.
    #[serde(default)]
    pub qubo_distance_km: Option<f64>,
    pub chosen: SequencingChoice,
    /// Why the shipped route was chosen, including why a QUBO tour was
    /// rejected when one was.
    pub reason: String,
    #[serde(default)]
    pub solver_runtime_ms: Option<f64>,
}

/// Builds the travelling-salesman model for one vehicle's stops.
///
/// The depot is fixed at both ends of the tour, so it needs no variable: the
/// first and last legs enter the objective as linear terms.
pub fn tsp_problem(
    network: &Network,
    depot: &str,
    stops: &[String],
) -> Result<OptimizationProblem> {
    if stops.len() < 3 {
        return Err(QuantumClawError::new(format!(
            "sequencing needs at least three stops to be a choice, got {}",
            stops.len()
        )));
    }

    let count = stops.len();
    let last = count - 1;
    let mut model = OptimizationProblem::minimize(format!("sequence-{depot}-{count}"))
        .with_metadata("domain", "logistics")
        .with_metadata("class", "sequencing")
        .with_metadata("depot", depot.to_string());

    for stop in stops {
        for position in 0..count {
            model.variables.push(
                BinaryVariable::new(visit_variable(stop, position))
                    .with_metadata("role", "visit")
                    .with_metadata("stop", stop.clone())
                    .with_metadata("position", position.to_string()),
            );
        }
    }

    // Depot legs: whichever stop lands first or last pays for the trip out of
    // and back to the depot.
    for stop in stops {
        model
            .linear
            .push(quantumclaw_ir::optimization::LinearTerm::new(
                visit_variable(stop, 0),
                network.distance_km(depot, stop),
            ));
        model
            .linear
            .push(quantumclaw_ir::optimization::LinearTerm::new(
                visit_variable(stop, last),
                network.distance_km(stop, depot),
            ));
    }

    // Consecutive legs: stop i at p followed by stop j at p+1 costs dist(i, j).
    for position in 0..last {
        for from in stops {
            for to in stops {
                if from == to {
                    continue;
                }
                let distance = network.distance_km(from, to);
                if distance != 0.0 {
                    model = model.with_interaction(
                        visit_variable(from, position),
                        visit_variable(to, position + 1),
                        distance,
                    );
                }
            }
        }
    }

    // Every stop is visited once...
    for stop in stops {
        model = model.with_constraint(OptimizationConstraint::exactly_one(
            format!("visit-{stop}-once"),
            (0..count).map(|position| visit_variable(stop, position)),
        ));
    }
    // ...and every position holds one stop.
    for position in 0..count {
        model = model.with_constraint(OptimizationConstraint::exactly_one(
            format!("position-{position}-filled"),
            stops
                .iter()
                .map(|stop| visit_variable(stop, position))
                .collect::<Vec<_>>(),
        ));
    }

    Ok(model)
}

/// Reads a visiting order out of a decoded solution.
///
/// Returns `None` when the sample is not a valid permutation — two stops in
/// one position, a position left empty, or a stop placed twice. Samplers
/// produce such states routinely, and turning one into a route would mean
/// shipping a plan that skips or duplicates a delivery.
pub fn decode_sequence(
    solution: &OptimizationSolution,
    model: &OptimizationProblem,
) -> Option<Vec<String>> {
    let mut placements: Vec<(usize, String)> = Vec::new();

    for name in &solution.selected {
        let variable = model.variable(name)?;
        if variable.metadata.get("role").map(String::as_str) != Some("visit") {
            continue;
        }
        let stop = variable.metadata.get("stop")?.clone();
        let position = variable.metadata.get("position")?.parse::<usize>().ok()?;
        placements.push((position, stop));
    }

    let expected = model
        .variables
        .iter()
        .filter_map(|variable| variable.metadata.get("stop"))
        .collect::<std::collections::BTreeSet<_>>()
        .len();

    if placements.len() != expected {
        return None;
    }

    placements.sort_by_key(|(position, _)| *position);
    // Positions must be 0..n with no gaps and no repeats.
    if placements
        .iter()
        .enumerate()
        .any(|(index, (position, _))| index != *position)
    {
        return None;
    }

    let sequence: Vec<String> = placements.into_iter().map(|(_, stop)| stop).collect();
    let unique: std::collections::BTreeSet<&String> = sequence.iter().collect();
    if unique.len() != sequence.len() {
        return None;
    }

    Some(sequence)
}

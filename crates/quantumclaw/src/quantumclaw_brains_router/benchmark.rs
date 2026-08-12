//! Comparing solvers on the metrics the business actually pays for.
//!
//! The customer's own plan is one of the candidates. That matters: an
//! optimization that cannot beat what dispatchers already do by hand has not
//! earned a rollout, whatever produced it.
//!
//! Stochastic samplers do not return the same answer twice, so a single run
//! per candidate is not a measurement — it is one draw from a distribution.
//! Every candidate is therefore run [`RouterBenchmark::repetitions`] times
//! with seeds derived from a fixed base, and ranked on the **median**
//! objective. Ranking on the best run would hand victory to whichever solver
//! is luckiest rather than whichever is best, and the same seed base always
//! reproduces the same report.

use crate::quantumclaw_brains::{BrainSolveContext, QuantumBrain};
use crate::quantumclaw_brains_router::brain::{QRouterBrain, QRouterRequest, QRouterResult};
use crate::quantumclaw_brains_router::constraints::solution_violations;
use crate::quantumclaw_brains_router::kpis::{self, KpiImprovement, RouterKpis};
use crate::quantumclaw_brains_router::models::RouteSolution;
use crate::quantumclaw_brains_router::network::Network;
use crate::quantumclaw_core::Result;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Spread of objective values across repeated runs of one candidate.
///
/// A wide spread is itself a finding: a solver whose median is good but whose
/// worst case is poor is a different operational proposition from one that is
/// consistent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkStats {
    pub runs: usize,
    pub feasible_runs: usize,
    pub objective_best: f64,
    pub objective_median: f64,
    pub objective_worst: f64,
    pub objective_mean: f64,
    pub objective_stddev: f64,
    /// Seeds used, so any single run can be reproduced on its own.
    pub seeds: Vec<u64>,
}

impl BenchmarkStats {
    /// Whether every run produced a feasible plan.
    pub fn always_feasible(&self) -> bool {
        self.runs > 0 && self.feasible_runs == self.runs
    }
}

/// One candidate plan and how it performed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkEntry {
    /// `baseline` for the customer's plan, otherwise the backend name.
    pub label: String,
    pub backend: String,
    pub kpis: RouterKpis,
    pub feasible: bool,
    pub runtime_ms: u64,
    pub solver_runtime_ms: Option<f64>,
    /// Spread across repeated runs. Absent for the customer baseline, which
    /// is a fixed plan rather than something a solver produced.
    #[serde(default)]
    pub stats: Option<BenchmarkStats>,
    /// KPI deltas against the baseline, when there is one.
    #[serde(default)]
    pub improvement: Option<KpiImprovement>,
    /// Populated when a candidate could not be evaluated at all.
    #[serde(default)]
    pub error: Option<String>,
}

/// The full comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterBenchmarkReport {
    pub problem_id: String,
    pub entries: Vec<BenchmarkEntry>,
    /// Best feasible candidate by objective value, if any candidate is feasible.
    pub winner: Option<String>,
    pub baseline_label: Option<String>,
    pub notes: Vec<String>,
}

impl RouterBenchmarkReport {
    pub fn entry(&self, label: &str) -> Option<&BenchmarkEntry> {
        self.entries.iter().find(|entry| entry.label == label)
    }
}

/// Runs one problem through several backends and the customer baseline.
#[derive(Debug, Clone)]
pub struct RouterBenchmark {
    pub brain: QRouterBrain,
    /// How many times each candidate runs. More than one is what turns a
    /// stochastic sampler's output into a measurement.
    pub repetitions: usize,
    /// Base seed. The same value reproduces the same report exactly.
    pub seed: u64,
}

impl Default for RouterBenchmark {
    fn default() -> Self {
        Self {
            brain: QRouterBrain::default(),
            repetitions: 5,
            seed: 1,
        }
    }
}

impl RouterBenchmark {
    pub fn new(brain: QRouterBrain) -> Self {
        Self {
            brain,
            ..Self::default()
        }
    }

    pub fn with_repetitions(mut self, repetitions: usize) -> Self {
        self.repetitions = repetitions.max(1);
        self
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Evaluates the customer's existing plan with the same KPI code every
    /// optimized plan uses, so the comparison is apples to apples.
    pub fn evaluate_baseline(
        &self,
        request: &QRouterRequest,
        baseline: &RouteSolution,
    ) -> Result<BenchmarkEntry> {
        let network = Network::build(&request.problem)?;
        let violations = solution_violations(&request.problem, &network, baseline);
        let kpis = kpis::evaluate(&request.problem, &network, baseline, 0, None);

        Ok(BenchmarkEntry {
            label: "baseline".into(),
            backend: "customer-historical".into(),
            feasible: violations.is_empty(),
            runtime_ms: 0,
            solver_runtime_ms: None,
            stats: None,
            improvement: None,
            error: None,
            kpis,
        })
    }

    /// Runs the brain once per backend and compares everything against the
    /// baseline when the problem carries one.
    pub async fn run(
        &self,
        request: QRouterRequest,
        backends: &[String],
        context: BrainSolveContext,
    ) -> Result<RouterBenchmarkReport> {
        let mut entries = Vec::new();
        let mut notes = Vec::new();

        let baseline_entry = match request.problem.baseline.clone() {
            Some(baseline) => {
                let entry = self.evaluate_baseline(&request, &baseline)?;
                entries.push(entry.clone());
                Some(entry)
            }
            None => {
                notes.push(
                    "the problem carries no customer baseline, so improvements are not reported"
                        .into(),
                );
                None
            }
        };

        for backend in backends {
            let started = Instant::now();
            let mut runs = Vec::new();
            let mut seeds = Vec::new();
            let mut failure = None;

            for repetition in 0..self.repetitions {
                let mut candidate = request.clone();
                // Name it explicitly. Leaving it unset would hand the choice
                // to the routing policy, which prefers a sampler.
                candidate.options.backend = Some(backend.clone());
                // Distinct, derived, and reproducible.
                let seed = self.seed.wrapping_add(repetition as u64);
                candidate.options.sampler_seed = Some(seed);
                seeds.push(seed);

                match self.brain.solve(candidate, context.clone()).await {
                    Ok(result) => runs.push(result),
                    Err(error) => {
                        failure = Some(error.to_string());
                        break;
                    }
                }
            }

            match failure {
                Some(error) => entries.push(BenchmarkEntry {
                    label: backend.clone(),
                    backend: backend.clone(),
                    kpis: empty_kpis(&request),
                    feasible: false,
                    runtime_ms: started.elapsed().as_millis() as u64,
                    solver_runtime_ms: None,
                    stats: None,
                    improvement: None,
                    error: Some(error),
                }),
                None => entries.push(self.entry_from_runs(backend, runs, seeds, &baseline_entry)),
            }
        }

        // Reliability first, then cost. A solver that is cheaper but only
        // sometimes produces a servable plan is not better than one that
        // always does, and a lucky best run is not a result — so candidates
        // are ranked on how often they were feasible, then on median
        // objective.
        let feasible_runs = |entry: &BenchmarkEntry| match &entry.stats {
            Some(stats) => stats.feasible_runs,
            None => usize::from(entry.feasible),
        };
        let objective = |entry: &BenchmarkEntry| match &entry.stats {
            Some(stats) => stats.objective_median,
            None => entry.kpis.objective_value,
        };

        let best = entries
            .iter()
            .filter(|entry| entry.error.is_none() && feasible_runs(entry) > 0)
            .max_by(|left, right| {
                feasible_runs(left)
                    .cmp(&feasible_runs(right))
                    .then_with(|| objective(right).total_cmp(&objective(left)))
            });

        let winner = best.map(|entry| entry.label.clone());
        match best {
            None => notes.push("no candidate produced a feasible plan".into()),
            Some(entry) => {
                if let Some(stats) = &entry.stats {
                    if !stats.always_feasible() {
                        notes.push(format!(
                            "the winner '{}' was feasible on only {} of {} runs; treat it as unreliable rather than best",
                            entry.label, stats.feasible_runs, stats.runs
                        ));
                    }
                }
            }
        }
        if self.repetitions == 1 {
            notes.push(
                "a single run per candidate cannot separate a solver's quality from its luck"
                    .into(),
            );
        }

        Ok(RouterBenchmarkReport {
            problem_id: request.problem.id.clone(),
            entries,
            winner,
            baseline_label: baseline_entry.map(|entry| entry.label),
            notes,
        })
    }

    /// Aggregates repeated runs into one entry.
    ///
    /// The reported KPIs come from the **median** run, so the headline numbers
    /// and the ranking statistic describe the same plan rather than two
    /// different ones.
    fn entry_from_runs(
        &self,
        label: &str,
        mut runs: Vec<QRouterResult>,
        seeds: Vec<u64>,
        baseline: &Option<BenchmarkEntry>,
    ) -> BenchmarkEntry {
        runs.sort_by(|left, right| {
            left.kpis
                .objective_value
                .total_cmp(&right.kpis.objective_value)
        });

        let objectives: Vec<f64> = runs.iter().map(|run| run.kpis.objective_value).collect();
        let count = objectives.len();
        let mean = objectives.iter().sum::<f64>() / count as f64;
        let variance = objectives
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / count as f64;
        let median_index = count / 2;

        let stats = BenchmarkStats {
            runs: count,
            feasible_runs: runs.iter().filter(|run| run.feasible).count(),
            objective_best: objectives[0],
            objective_median: objectives[median_index],
            objective_worst: objectives[count - 1],
            objective_mean: mean,
            objective_stddev: variance.sqrt(),
            seeds,
        };

        let representative = runs.swap_remove(median_index);
        let improvement = baseline
            .as_ref()
            .map(|entry| representative.kpis.improvement_over(&entry.kpis));
        let backend = representative
            .subproblems
            .first()
            .map(|report| report.backend.clone())
            .unwrap_or_else(|| label.to_string());

        BenchmarkEntry {
            label: label.to_string(),
            backend,
            feasible: representative.feasible,
            runtime_ms: representative.runtime_ms,
            solver_runtime_ms: representative.kpis.solver_runtime_ms,
            stats: Some(stats),
            improvement,
            error: None,
            kpis: representative.kpis,
        }
    }
}

/// Zeroed KPIs for a candidate that failed before producing a plan.
fn empty_kpis(request: &QRouterRequest) -> RouterKpis {
    RouterKpis {
        total_distance_km: 0.0,
        total_travel_time_min: 0.0,
        total_service_time_min: 0.0,
        vehicles_used: 0,
        vehicles_available: request.problem.vehicles.len(),
        fleet_utilization: 0.0,
        capacity_utilization: 0.0,
        deliveries_served: 0,
        unassigned_deliveries: request.problem.deliveries.len(),
        late_deliveries: 0,
        sla_violation_minutes: 0.0,
        sla_breaches: 0,
        estimated_fuel_liters: 0.0,
        estimated_co2_kg: 0.0,
        estimated_operating_cost: 0.0,
        objective_value: f64::INFINITY,
        feasible: false,
        constraint_violations: request.problem.deliveries.len(),
        optimization_runtime_ms: 0,
        solver_runtime_ms: None,
    }
}

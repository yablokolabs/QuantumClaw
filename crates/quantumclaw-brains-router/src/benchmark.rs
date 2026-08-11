//! Comparing solvers on the metrics the business actually pays for.
//!
//! The customer's own plan is one of the candidates. That matters: an
//! optimization that cannot beat what dispatchers already do by hand has not
//! earned a rollout, whatever produced it.

use crate::brain::{QRouterBrain, QRouterRequest, QRouterResult};
use crate::constraints::solution_violations;
use crate::kpis::{self, KpiImprovement, RouterKpis};
use crate::models::RouteSolution;
use crate::network::Network;
use quantumclaw_brains::{BrainSolveContext, QuantumBrain};
use quantumclaw_core::Result;
use serde::{Deserialize, Serialize};
use std::time::Instant;

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
#[derive(Debug, Clone, Default)]
pub struct RouterBenchmark {
    pub brain: QRouterBrain,
}

impl RouterBenchmark {
    pub fn new(brain: QRouterBrain) -> Self {
        Self { brain }
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
            let mut candidate = request.clone();
            candidate.options.backend = if backend == "classical" {
                None
            } else {
                Some(backend.clone())
            };

            let started = Instant::now();
            match self.brain.solve(candidate, context.clone()).await {
                Ok(result) => {
                    entries.push(self.entry_from_result(backend, result, &baseline_entry))
                }
                Err(error) => entries.push(BenchmarkEntry {
                    label: backend.clone(),
                    backend: backend.clone(),
                    kpis: empty_kpis(&request),
                    feasible: false,
                    runtime_ms: started.elapsed().as_millis() as u64,
                    solver_runtime_ms: None,
                    improvement: None,
                    error: Some(error.to_string()),
                }),
            }
        }

        // Only a feasible plan can win. A cheaper infeasible plan is not a plan.
        let winner = entries
            .iter()
            .filter(|entry| entry.feasible && entry.error.is_none())
            .min_by(|left, right| {
                left.kpis
                    .objective_value
                    .total_cmp(&right.kpis.objective_value)
            })
            .map(|entry| entry.label.clone());

        if winner.is_none() {
            notes.push("no candidate produced a feasible plan".into());
        }

        Ok(RouterBenchmarkReport {
            problem_id: request.problem.id.clone(),
            entries,
            winner,
            baseline_label: baseline_entry.map(|entry| entry.label),
            notes,
        })
    }

    fn entry_from_result(
        &self,
        label: &str,
        result: QRouterResult,
        baseline: &Option<BenchmarkEntry>,
    ) -> BenchmarkEntry {
        let improvement = baseline
            .as_ref()
            .map(|entry| result.kpis.improvement_over(&entry.kpis));
        let backend = result
            .subproblems
            .first()
            .map(|report| report.backend.clone())
            .unwrap_or_else(|| label.to_string());

        BenchmarkEntry {
            label: label.to_string(),
            backend,
            feasible: result.feasible,
            runtime_ms: result.runtime_ms,
            solver_runtime_ms: result.kpis.solver_runtime_ms,
            improvement,
            error: None,
            kpis: result.kpis,
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

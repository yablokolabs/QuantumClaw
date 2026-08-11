//! Logistics KPI evaluation.
//!
//! Which solver produced a plan matters far less than what the plan costs to
//! run. These are the numbers an operations team already reports on, computed
//! the same way for every candidate so comparisons mean something.

use crate::quantumclaw_brains::KpiReport;
use crate::quantumclaw_brains_router::constraints::evaluate_solution;
use crate::quantumclaw_brains_router::constraints::{
    solution_violations, unserved_deliveries, RouteEvaluation, ViolationKind,
};
use crate::quantumclaw_brains_router::models::{DeliveryProblem, RouteSolution};
use crate::quantumclaw_brains_router::network::Network;
use serde::{Deserialize, Serialize};

/// Operational metrics for one plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterKpis {
    pub total_distance_km: f64,
    pub total_travel_time_min: f64,
    pub total_service_time_min: f64,
    pub vehicles_used: usize,
    pub vehicles_available: usize,
    /// Share of the fleet that leaves the depot.
    pub fleet_utilization: f64,
    /// Load carried against the capacity of the vehicles actually used.
    pub capacity_utilization: f64,
    pub deliveries_served: usize,
    pub unassigned_deliveries: usize,
    pub late_deliveries: usize,
    pub sla_violation_minutes: f64,
    pub sla_breaches: usize,
    pub estimated_fuel_liters: f64,
    pub estimated_co2_kg: f64,
    pub estimated_operating_cost: f64,
    /// Total operating cost including SLA penalties: the business objective.
    pub objective_value: f64,
    pub feasible: bool,
    pub constraint_violations: usize,
    /// Wall time of the optimization that produced this plan.
    pub optimization_runtime_ms: u64,
    /// Time spent inside solver backends, when they report it.
    pub solver_runtime_ms: Option<f64>,
}

impl RouterKpis {
    /// Exports metrics into the domain-neutral report brains return.
    pub fn to_report(&self) -> KpiReport {
        let mut report = KpiReport::default();
        report.set("total_distance_km", self.total_distance_km);
        report.set("total_travel_time_min", self.total_travel_time_min);
        report.set("total_service_time_min", self.total_service_time_min);
        report.set("vehicles_used", self.vehicles_used as f64);
        report.set("vehicles_available", self.vehicles_available as f64);
        report.set("fleet_utilization", self.fleet_utilization);
        report.set("capacity_utilization", self.capacity_utilization);
        report.set("deliveries_served", self.deliveries_served as f64);
        report.set("unassigned_deliveries", self.unassigned_deliveries as f64);
        report.set("late_deliveries", self.late_deliveries as f64);
        report.set("sla_violation_minutes", self.sla_violation_minutes);
        report.set("sla_breaches", self.sla_breaches as f64);
        report.set("estimated_fuel_liters", self.estimated_fuel_liters);
        report.set("estimated_co2_kg", self.estimated_co2_kg);
        report.set("estimated_operating_cost", self.estimated_operating_cost);
        report.set("objective_value", self.objective_value);
        report.set("feasible", f64::from(u8::from(self.feasible)));
        report.set(
            "optimization_runtime_ms",
            self.optimization_runtime_ms as f64,
        );
        if let Some(solver_runtime_ms) = self.solver_runtime_ms {
            report.set("solver_runtime_ms", solver_runtime_ms);
        }
        report
    }

    /// Improvement of this plan over a baseline, as a fraction of the baseline.
    ///
    /// Positive means this plan is better. Returns `None` when the baseline
    /// metric is zero and a ratio would be meaningless.
    pub fn improvement_over(&self, baseline: &Self) -> KpiImprovement {
        KpiImprovement {
            distance_km_saved: baseline.total_distance_km - self.total_distance_km,
            distance_improvement: ratio(
                baseline.total_distance_km - self.total_distance_km,
                baseline.total_distance_km,
            ),
            cost_saved: baseline.objective_value - self.objective_value,
            cost_improvement: ratio(
                baseline.objective_value - self.objective_value,
                baseline.objective_value,
            ),
            co2_kg_saved: baseline.estimated_co2_kg - self.estimated_co2_kg,
            fuel_liters_saved: baseline.estimated_fuel_liters - self.estimated_fuel_liters,
            vehicles_saved: baseline.vehicles_used as i64 - self.vehicles_used as i64,
            late_deliveries_avoided: baseline.late_deliveries as i64 - self.late_deliveries as i64,
        }
    }
}

fn ratio(delta: f64, baseline: f64) -> Option<f64> {
    if baseline.abs() <= f64::EPSILON {
        return None;
    }
    Some(delta / baseline.abs())
}

/// KPI deltas against a baseline plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KpiImprovement {
    pub distance_km_saved: f64,
    pub distance_improvement: Option<f64>,
    pub cost_saved: f64,
    pub cost_improvement: Option<f64>,
    pub co2_kg_saved: f64,
    pub fuel_liters_saved: f64,
    pub vehicles_saved: i64,
    pub late_deliveries_avoided: i64,
}

/// Computes KPIs for a plan.
pub fn evaluate(
    problem: &DeliveryProblem,
    network: &Network,
    solution: &RouteSolution,
    optimization_runtime_ms: u64,
    solver_runtime_ms: Option<f64>,
) -> RouterKpis {
    let evaluations = evaluate_solution(problem, network, solution);
    let violations = solution_violations(problem, network, solution);
    let unassigned = unserved_deliveries(problem, solution);

    let mut total_distance_km = 0.0;
    let mut total_travel_time_min = 0.0;
    let mut total_service_time_min = 0.0;
    let mut late_deliveries = 0usize;
    let mut sla_violation_minutes = 0.0;
    let mut fuel_liters = 0.0;
    let mut co2_kg = 0.0;
    let mut operating_cost = 0.0;
    let mut load_used = 0u32;
    let mut capacity_used = 0u32;

    for evaluation in &evaluations {
        let Some(vehicle) = problem.vehicle(&evaluation.vehicle_id) else {
            continue;
        };
        if evaluation.load == 0 && evaluation.distance_km == 0.0 {
            continue;
        }

        total_distance_km += evaluation.distance_km;
        total_travel_time_min += evaluation.travel_time_min;
        total_service_time_min += evaluation.service_time_min;
        late_deliveries += evaluation.late_deliveries;
        sla_violation_minutes += evaluation.lateness_minutes;
        load_used += evaluation.load;
        capacity_used += vehicle.capacity;

        let route_fuel = evaluation.distance_km * vehicle.fuel_l_per_100km / 100.0;
        fuel_liters += route_fuel;
        co2_kg += evaluation.distance_km * vehicle.co2_g_per_km / 1000.0;
        operating_cost += vehicle.fixed_cost
            + evaluation.distance_km * vehicle.cost_per_km
            + route_fuel * problem.cost_model.fuel_price_per_liter
            + evaluation.total_time_min() / 60.0 * problem.cost_model.driver_cost_per_hour;
    }

    let sla_breaches = evaluations
        .iter()
        .flat_map(|evaluation: &RouteEvaluation| evaluation.violations.iter())
        .filter(|violation| {
            violation.kind == ViolationKind::TimeWindow
                && violation.magnitude > problem.sla.breach_after_minutes
        })
        .count();

    let sla_cost = sla_violation_minutes * problem.sla.late_penalty_per_minute;
    let vehicles_used = solution.vehicles_used();
    let vehicles_available = problem.vehicles.len();
    let deliveries_served = problem.deliveries.len() - unassigned.len();

    RouterKpis {
        total_distance_km,
        total_travel_time_min,
        total_service_time_min,
        vehicles_used,
        vehicles_available,
        fleet_utilization: if vehicles_available == 0 {
            0.0
        } else {
            vehicles_used as f64 / vehicles_available as f64
        },
        capacity_utilization: if capacity_used == 0 {
            0.0
        } else {
            f64::from(load_used) / f64::from(capacity_used)
        },
        deliveries_served,
        unassigned_deliveries: unassigned.len(),
        late_deliveries,
        sla_violation_minutes,
        sla_breaches,
        estimated_fuel_liters: fuel_liters,
        estimated_co2_kg: co2_kg,
        estimated_operating_cost: operating_cost,
        objective_value: operating_cost + sla_cost,
        feasible: violations.is_empty(),
        constraint_violations: violations.len(),
        optimization_runtime_ms,
        solver_runtime_ms,
    }
}

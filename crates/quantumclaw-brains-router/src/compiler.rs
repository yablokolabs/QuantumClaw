//! Turns a logistics subproblem into a domain-neutral optimization model.
//!
//! This is the only place where routing vocabulary becomes binary variables.
//! What comes out is an ordinary [`OptimizationProblem`] that any backend can
//! consume; the QUBO mechanics live in `quantumclaw-optimization`.

use crate::decomposition::Subproblem;
use crate::models::DeliveryProblem;
use crate::network::Network;
use quantumclaw_core::{QuantumClawError, Result};
use quantumclaw_ir::optimization::{
    BinaryVariable, LinearTerm, OptimizationConstraint, OptimizationProblem,
};

/// Prefix of a delivery-to-vehicle assignment variable.
pub const ASSIGN_PREFIX: &str = "assign";
/// Prefix of a vehicle-in-use variable.
pub const USE_PREFIX: &str = "use";

/// Weights that shape the assignment objective.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentWeights {
    /// Multiplier on the out-and-back travel cost of serving a stop.
    pub travel_weight: f64,
    /// Multiplier on a vehicle's fixed cost when it is used at all.
    pub fixed_cost_weight: f64,
    /// Discount applied per unit of delivery priority, so urgent stops are
    /// preferred when capacity is scarce.
    pub priority_weight: f64,
    /// Multiplier on the distance between two stops carried by the same
    /// vehicle. This is what makes the model prefer tight clusters, and it is
    /// the term that gives the problem genuine quadratic structure — without
    /// it, a single-depot instance has nothing for a sampler to search.
    pub proximity_weight: f64,
}

impl Default for AssignmentWeights {
    fn default() -> Self {
        Self {
            travel_weight: 1.0,
            fixed_cost_weight: 1.0,
            priority_weight: 0.0,
            proximity_weight: 0.5,
        }
    }
}

/// Name of the variable that assigns `delivery` to `vehicle`.
pub fn assignment_variable(delivery: &str, vehicle: &str) -> String {
    format!("{ASSIGN_PREFIX}::{delivery}::{vehicle}")
}

/// Name of the variable that marks `vehicle` as used.
pub fn use_variable(vehicle: &str) -> String {
    format!("{USE_PREFIX}::{vehicle}")
}

/// Builds the vehicle-assignment model for one subproblem.
///
/// Variables:
/// * one binary per feasible (delivery, vehicle) pair,
/// * one binary per vehicle recording whether it leaves the depot.
///
/// Constraints:
/// * every delivery is assigned exactly once,
/// * no vehicle is loaded past its capacity,
/// * assigning any delivery to a vehicle marks that vehicle as used.
pub fn assignment_problem(
    problem: &DeliveryProblem,
    network: &Network,
    subproblem: &Subproblem,
    weights: &AssignmentWeights,
) -> Result<OptimizationProblem> {
    if subproblem.delivery_ids.is_empty() {
        return Err(QuantumClawError::new(format!(
            "subproblem '{}' has no deliveries to assign",
            subproblem.id
        )));
    }
    if subproblem.vehicle_ids.is_empty() {
        return Err(QuantumClawError::new(format!(
            "subproblem '{}' has no vehicles available at depot '{}'",
            subproblem.id, subproblem.depot_id
        )));
    }

    let mut model = OptimizationProblem::minimize(subproblem.id.clone())
        .with_metadata("domain", "logistics")
        .with_metadata("class", subproblem.class.as_str())
        .with_metadata("depot", subproblem.depot_id.clone());

    for vehicle_id in &subproblem.vehicle_ids {
        let vehicle = problem
            .vehicle(vehicle_id)
            .ok_or_else(|| QuantumClawError::new(format!("unknown vehicle '{vehicle_id}'")))?;
        let name = use_variable(vehicle_id);
        model.variables.push(
            BinaryVariable::new(name.clone())
                .with_metadata("role", "vehicle-use")
                .with_metadata("vehicle", vehicle_id.clone()),
        );
        model.linear.push(LinearTerm::new(
            name,
            vehicle.fixed_cost * weights.fixed_cost_weight,
        ));
    }

    for delivery_id in &subproblem.delivery_ids {
        let delivery = problem
            .delivery(delivery_id)
            .ok_or_else(|| QuantumClawError::new(format!("unknown delivery '{delivery_id}'")))?;
        let mut choices = Vec::new();

        for vehicle_id in &subproblem.vehicle_ids {
            let vehicle = problem
                .vehicle(vehicle_id)
                .ok_or_else(|| QuantumClawError::new(format!("unknown vehicle '{vehicle_id}'")))?;
            // A vehicle that cannot carry the load is not a choice at all.
            if vehicle.capacity < delivery.demand {
                continue;
            }

            let name = assignment_variable(delivery_id, vehicle_id);
            let out_and_back = 2.0 * network.distance_km(&vehicle.depot_id, delivery_id);
            let cost = out_and_back * vehicle.cost_per_km * weights.travel_weight
                - delivery.priority * weights.priority_weight;

            model.variables.push(
                BinaryVariable::new(name.clone())
                    .with_metadata("role", "assignment")
                    .with_metadata("delivery", delivery_id.clone())
                    .with_metadata("vehicle", vehicle_id.clone()),
            );
            model.linear.push(LinearTerm::new(name.clone(), cost));
            choices.push(name);
        }

        if choices.is_empty() {
            return Err(QuantumClawError::new(format!(
                "delivery '{delivery_id}' needs {} units but no vehicle in subproblem '{}' can carry it",
                delivery.demand, subproblem.id
            )));
        }

        model = model.with_constraint(OptimizationConstraint::exactly_one(
            format!("serve-{delivery_id}"),
            choices,
        ));
    }

    // Two stops served by the same vehicle cost what it takes to travel
    // between them, so clustered assignments score better than scattered ones.
    if weights.proximity_weight != 0.0 {
        for vehicle_id in &subproblem.vehicle_ids {
            let Some(vehicle) = problem.vehicle(vehicle_id) else {
                continue;
            };
            for (index, first) in subproblem.delivery_ids.iter().enumerate() {
                for second in subproblem.delivery_ids.iter().skip(index + 1) {
                    let (left, right) = (
                        assignment_variable(first, vehicle_id),
                        assignment_variable(second, vehicle_id),
                    );
                    let declared = |name: &String| {
                        model
                            .variables
                            .iter()
                            .any(|variable| &variable.name == name)
                    };
                    if !declared(&left) || !declared(&right) {
                        continue;
                    }
                    let coefficient = network.distance_km(first, second)
                        * vehicle.cost_per_km
                        * weights.proximity_weight;
                    if coefficient != 0.0 {
                        model = model.with_interaction(left, right, coefficient);
                    }
                }
            }
        }
    }

    for vehicle_id in &subproblem.vehicle_ids {
        let vehicle = problem
            .vehicle(vehicle_id)
            .ok_or_else(|| QuantumClawError::new(format!("unknown vehicle '{vehicle_id}'")))?;

        let mut capacity_terms = Vec::new();
        for delivery_id in &subproblem.delivery_ids {
            let name = assignment_variable(delivery_id, vehicle_id);
            if !model.variables.iter().any(|variable| variable.name == name) {
                continue;
            }
            let demand = problem
                .delivery(delivery_id)
                .map(|delivery| delivery.demand)
                .unwrap_or(0);
            capacity_terms.push(LinearTerm::new(name.clone(), f64::from(demand)));

            model = model.with_constraint(OptimizationConstraint::implication(
                format!("use-{vehicle_id}-for-{delivery_id}"),
                name,
                use_variable(vehicle_id),
            ));
        }

        if !capacity_terms.is_empty() {
            model = model.with_constraint(OptimizationConstraint::linear_at_most(
                format!("capacity-{vehicle_id}"),
                capacity_terms,
                f64::from(vehicle.capacity),
            ));
        }
    }

    Ok(model)
}

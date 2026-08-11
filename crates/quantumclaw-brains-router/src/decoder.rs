//! Turns solver output back into routes a dispatcher can run.
//!
//! Samplers return assignments, not plans. This module reconstructs routes,
//! repairs whatever the sampler got wrong (over-loaded vehicles, deliveries
//! nobody took), sequences each vehicle's stops classically, and hands back
//! something that has been re-checked against the domain rules.

use crate::models::{DeliveryProblem, Route, RouteSolution};
use crate::network::Network;
use crate::vrp;
use quantumclaw_ir::optimization::OptimizationSolution;
use std::collections::BTreeMap;

/// Delivery id to vehicle id.
pub type Assignment = BTreeMap<String, String>;

/// Reads assignments out of a decoded optimization solution.
///
/// Variables carry their domain meaning in metadata, so decoding never parses
/// variable names.
pub fn assignments_from_solution(
    solution: &OptimizationSolution,
    model: &quantumclaw_ir::optimization::OptimizationProblem,
) -> Assignment {
    let mut assignment = Assignment::new();
    for name in &solution.selected {
        let Some(variable) = model.variable(name) else {
            continue;
        };
        if variable.metadata.get("role").map(String::as_str) != Some("assignment") {
            continue;
        }
        let (Some(delivery), Some(vehicle)) = (
            variable.metadata.get("delivery"),
            variable.metadata.get("vehicle"),
        ) else {
            continue;
        };
        // A sampler can select two vehicles for one delivery; the first wins
        // and the repair pass rebalances afterwards.
        assignment
            .entry(delivery.clone())
            .or_insert_with(|| vehicle.clone());
    }
    assignment
}

/// Assigns every delivery to the cheapest vehicle that can still carry it.
///
/// This is the classical fallback used when no solver backend is available,
/// and the starting point the repair pass improves.
pub fn greedy_assignment(
    problem: &DeliveryProblem,
    network: &Network,
    deliveries: &[String],
    vehicles: &[String],
) -> Assignment {
    let mut loads: BTreeMap<String, u32> = vehicles
        .iter()
        .map(|vehicle| (vehicle.clone(), 0))
        .collect();
    let mut assignment = Assignment::new();

    // Heaviest first: large loads are the hardest to place late.
    let mut ordered: Vec<&String> = deliveries.iter().collect();
    ordered.sort_by(|left, right| {
        let demand = |id: &str| problem.delivery(id).map(|d| d.demand).unwrap_or(0);
        demand(right)
            .cmp(&demand(left))
            .then_with(|| left.cmp(right))
    });

    let mut carried: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for delivery_id in ordered {
        let Some(delivery) = problem.delivery(delivery_id) else {
            continue;
        };
        let best = vehicles
            .iter()
            .filter_map(|vehicle_id| {
                let vehicle = problem.vehicle(vehicle_id)?;
                let load = loads.get(vehicle_id).copied().unwrap_or(0);
                if load + delivery.demand > vehicle.capacity {
                    return None;
                }
                Some((
                    vehicle_id.clone(),
                    insertion_cost(
                        network,
                        &vehicle.depot_id,
                        vehicle.cost_per_km,
                        carried.get(vehicle_id).map(Vec::as_slice).unwrap_or(&[]),
                        delivery_id,
                    ),
                ))
            })
            .min_by(|left, right| left.1.total_cmp(&right.1));

        if let Some((vehicle_id, _)) = best {
            *loads.entry(vehicle_id.clone()).or_insert(0) += delivery.demand;
            carried
                .entry(vehicle_id.clone())
                .or_default()
                .push(delivery_id.clone());
            assignment.insert(delivery_id.clone(), vehicle_id);
        }
    }

    assignment
}

/// Marginal cost of adding a stop to a vehicle's tour.
///
/// Cheapest insertion: the extra distance of splicing the stop into the best
/// position of the existing tour, or the full out-and-back trip when the
/// vehicle is still empty. This is what makes the classical fallback cluster
/// rather than scatter, and it keeps the classical baseline honest when it is
/// benchmarked against a sampler.
fn insertion_cost(
    network: &Network,
    depot_id: &str,
    cost_per_km: f64,
    existing: &[String],
    delivery_id: &str,
) -> f64 {
    if existing.is_empty() {
        return 2.0 * network.distance_km(depot_id, delivery_id) * cost_per_km;
    }

    // The tour runs depot -> existing... -> depot; try every gap.
    let mut best = f64::INFINITY;
    for position in 0..=existing.len() {
        let before = if position == 0 {
            depot_id
        } else {
            &existing[position - 1]
        };
        let after = if position == existing.len() {
            depot_id
        } else {
            &existing[position]
        };
        let delta = network.distance_km(before, delivery_id)
            + network.distance_km(delivery_id, after)
            - network.distance_km(before, after);
        best = best.min(delta);
    }

    best * cost_per_km
}

/// Fixes over-capacity vehicles and places deliveries the solver dropped.
pub fn repair(
    problem: &DeliveryProblem,
    network: &Network,
    vehicles: &[String],
    assignment: Assignment,
) -> (Assignment, Vec<String>) {
    let mut repaired = Assignment::new();
    let mut loads: BTreeMap<String, u32> = vehicles
        .iter()
        .map(|vehicle| (vehicle.clone(), 0))
        .collect();
    let mut displaced: Vec<String> = Vec::new();

    // Keep what fits, in a deterministic order.
    for (delivery_id, vehicle_id) in assignment {
        let (Some(delivery), Some(vehicle)) =
            (problem.delivery(&delivery_id), problem.vehicle(&vehicle_id))
        else {
            displaced.push(delivery_id);
            continue;
        };
        let load = loads.get(&vehicle_id).copied().unwrap_or(0);
        if load + delivery.demand <= vehicle.capacity {
            loads.insert(vehicle_id.clone(), load + delivery.demand);
            repaired.insert(delivery_id, vehicle_id);
        } else {
            displaced.push(delivery_id);
        }
    }

    // Deliveries nobody took, plus the ones bumped above.
    let mut unplaced: Vec<String> = problem
        .deliveries
        .iter()
        .map(|delivery| delivery.id.clone())
        .filter(|id| !repaired.contains_key(id))
        .collect();
    for delivery_id in displaced {
        if !unplaced.contains(&delivery_id) {
            unplaced.push(delivery_id);
        }
    }

    let mut unassigned = Vec::new();
    for delivery_id in unplaced {
        let Some(delivery) = problem.delivery(&delivery_id) else {
            continue;
        };
        let best = vehicles
            .iter()
            .filter_map(|vehicle_id| {
                let vehicle = problem.vehicle(vehicle_id)?;
                let load = loads.get(vehicle_id).copied().unwrap_or(0);
                if load + delivery.demand > vehicle.capacity {
                    return None;
                }
                let cost = 2.0
                    * network.distance_km(&vehicle.depot_id, &delivery_id)
                    * vehicle.cost_per_km;
                Some((vehicle_id.clone(), cost))
            })
            .min_by(|left, right| left.1.total_cmp(&right.1));

        match best {
            Some((vehicle_id, _)) => {
                *loads.entry(vehicle_id.clone()).or_insert(0) += delivery.demand;
                repaired.insert(delivery_id, vehicle_id);
            }
            None => unassigned.push(delivery_id),
        }
    }

    (repaired, unassigned)
}

/// Sequences each vehicle's stops and produces the final plan.
pub fn build_solution(
    problem: &DeliveryProblem,
    network: &Network,
    assignment: &Assignment,
    unassigned: Vec<String>,
) -> RouteSolution {
    let mut by_vehicle: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (delivery_id, vehicle_id) in assignment {
        by_vehicle
            .entry(vehicle_id.clone())
            .or_default()
            .push(delivery_id.clone());
    }

    let mut solution = RouteSolution::new(problem.id.clone());
    solution.unassigned = unassigned;

    for (vehicle_id, stops) in by_vehicle {
        let Some(vehicle) = problem.vehicle(&vehicle_id) else {
            continue;
        };
        let ordered = vrp::sequence(network, &vehicle.depot_id, &stops);
        solution.routes.push(Route {
            vehicle_id: vehicle_id.clone(),
            depot_id: vehicle.depot_id.clone(),
            stops: ordered,
        });
    }

    solution
}

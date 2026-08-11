//! Logistics feasibility rules.
//!
//! These evaluate a concrete plan. They are the domain counterpart of the
//! penalty constraints the optimization layer compiles into a QUBO: the QUBO
//! steers the search, these decide what the business will accept.

use crate::models::{DeliveryProblem, Route, RouteSolution};
use crate::network::Network;
use serde::{Deserialize, Serialize};

/// A rule a plan breaks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterViolation {
    pub kind: ViolationKind,
    pub subject: String,
    pub description: String,
    /// How far past the limit, in the rule's own units.
    pub magnitude: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationKind {
    Capacity,
    TimeWindow,
    MaxDistance,
    Shift,
    UnassignedDelivery,
    UnknownReference,
    DepotMismatch,
}

/// What one route costs and where it breaks the rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteEvaluation {
    pub vehicle_id: String,
    pub distance_km: f64,
    pub travel_time_min: f64,
    pub service_time_min: f64,
    pub load: u32,
    pub capacity: u32,
    pub late_deliveries: usize,
    pub lateness_minutes: f64,
    pub violations: Vec<RouterViolation>,
}

impl RouteEvaluation {
    pub fn total_time_min(&self) -> f64 {
        self.travel_time_min + self.service_time_min
    }

    pub fn feasible(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Evaluates a single route against capacity, time windows, and shift limits.
pub fn evaluate_route(
    problem: &DeliveryProblem,
    network: &Network,
    route: &Route,
) -> RouteEvaluation {
    let mut violations = Vec::new();
    let vehicle = problem.vehicle(&route.vehicle_id);
    let capacity = vehicle.map(|vehicle| vehicle.capacity).unwrap_or(0);

    if vehicle.is_none() {
        violations.push(RouterViolation {
            kind: ViolationKind::UnknownReference,
            subject: route.vehicle_id.clone(),
            description: format!("route references unknown vehicle '{}'", route.vehicle_id),
            magnitude: 1.0,
        });
    }

    let mut load = 0u32;
    let mut service_time_min = 0.0;
    let mut clock = vehicle
        .and_then(|vehicle| vehicle.shift)
        .map(|shift| shift.start_min)
        .unwrap_or(0.0);
    let mut previous = route.depot_id.clone();
    let mut late_deliveries = 0usize;
    let mut lateness_minutes = 0.0;

    for stop in &route.stops {
        let Some(delivery) = problem.delivery(stop) else {
            violations.push(RouterViolation {
                kind: ViolationKind::UnknownReference,
                subject: stop.clone(),
                description: format!("route references unknown delivery '{stop}'"),
                magnitude: 1.0,
            });
            continue;
        };

        load += delivery.demand;
        clock += network.duration_min(&previous, stop);
        if let Some(window) = delivery.window {
            // Arriving early means waiting, not a violation.
            if clock < window.start_min {
                clock = window.start_min;
            }
            let lateness = window.lateness(clock);
            if lateness > 0.0 {
                late_deliveries += 1;
                lateness_minutes += lateness;
                violations.push(RouterViolation {
                    kind: ViolationKind::TimeWindow,
                    subject: delivery.id.clone(),
                    description: format!(
                        "arrives {lateness:.1} minutes after the delivery window closes"
                    ),
                    magnitude: lateness,
                });
            }
        }
        clock += delivery.service_time_min;
        service_time_min += delivery.service_time_min;

        if let Some(required_depot) = &delivery.depot_id {
            if required_depot != &route.depot_id {
                violations.push(RouterViolation {
                    kind: ViolationKind::DepotMismatch,
                    subject: delivery.id.clone(),
                    description: format!(
                        "delivery is restricted to depot '{required_depot}' but is served from '{}'",
                        route.depot_id
                    ),
                    magnitude: 1.0,
                });
            }
        }

        previous = stop.clone();
    }

    let distance_km = network.route_distance_km(&route.depot_id, &route.stops);
    let travel_time_min = network.route_duration_min(&route.depot_id, &route.stops);

    if load > capacity {
        violations.push(RouterViolation {
            kind: ViolationKind::Capacity,
            subject: route.vehicle_id.clone(),
            description: format!("carries {load} units against a capacity of {capacity}"),
            magnitude: f64::from(load - capacity),
        });
    }

    if let Some(vehicle) = vehicle {
        if let Some(limit) = vehicle.max_distance_km {
            if distance_km > limit {
                violations.push(RouterViolation {
                    kind: ViolationKind::MaxDistance,
                    subject: vehicle.id.clone(),
                    description: format!(
                        "route is {distance_km:.1} km against a limit of {limit:.1} km"
                    ),
                    magnitude: distance_km - limit,
                });
            }
        }
        if let Some(shift) = vehicle.shift {
            let finish = clock + network.duration_min(&previous, &route.depot_id);
            if finish > shift.end_min {
                violations.push(RouterViolation {
                    kind: ViolationKind::Shift,
                    subject: vehicle.id.clone(),
                    description: format!(
                        "returns {:.1} minutes after the shift ends",
                        finish - shift.end_min
                    ),
                    magnitude: finish - shift.end_min,
                });
            }
        }
    }

    RouteEvaluation {
        vehicle_id: route.vehicle_id.clone(),
        distance_km,
        travel_time_min,
        service_time_min,
        load,
        capacity,
        late_deliveries,
        lateness_minutes,
        violations,
    }
}

/// Evaluates every route plus plan-level rules such as unserved deliveries.
pub fn evaluate_solution(
    problem: &DeliveryProblem,
    network: &Network,
    solution: &RouteSolution,
) -> Vec<RouteEvaluation> {
    solution
        .routes
        .iter()
        .map(|route| evaluate_route(problem, network, route))
        .collect()
}

/// Deliveries that appear in no route.
pub fn unserved_deliveries(problem: &DeliveryProblem, solution: &RouteSolution) -> Vec<String> {
    let served: Vec<&String> = solution.served_deliveries();
    problem
        .deliveries
        .iter()
        .filter(|delivery| !served.contains(&&delivery.id))
        .map(|delivery| delivery.id.clone())
        .collect()
}

/// Every violation in a plan, including deliveries nobody serves.
pub fn solution_violations(
    problem: &DeliveryProblem,
    network: &Network,
    solution: &RouteSolution,
) -> Vec<RouterViolation> {
    let mut violations: Vec<RouterViolation> = evaluate_solution(problem, network, solution)
        .into_iter()
        .flat_map(|evaluation| evaluation.violations)
        .collect();

    for delivery in unserved_deliveries(problem, solution) {
        violations.push(RouterViolation {
            kind: ViolationKind::UnassignedDelivery,
            subject: delivery.clone(),
            description: format!("delivery '{delivery}' is not served by any vehicle"),
            magnitude: 1.0,
        });
    }

    violations
}

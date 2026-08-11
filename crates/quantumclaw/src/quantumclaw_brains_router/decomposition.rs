//! Decomposition strategies for large instances.
//!
//! A 10,000-delivery problem is not one QUBO. Q-Router splits it into pieces
//! small enough that a solver — classical or quantum — can say something useful
//! about each, then reassembles the answers. Every strategy must produce a
//! partition: each delivery appears in exactly one subproblem.

use crate::quantumclaw_brains::{Decomposition, SubproblemSummary};
use crate::quantumclaw_brains_router::models::DeliveryProblem;
use crate::quantumclaw_brains_router::network::Network;
use crate::quantumclaw_core::{QuantumClawError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What kind of combinatorial question a subproblem asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubproblemClass {
    /// Which vehicle serves which delivery.
    VehicleAssignment,
    /// In what order one vehicle visits its stops.
    Sequencing,
    /// Which deliveries belong together.
    Clustering,
}

impl SubproblemClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::VehicleAssignment => "vehicle-assignment",
            Self::Sequencing => "sequencing",
            Self::Clustering => "clustering",
        }
    }
}

/// One piece of a decomposed instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subproblem {
    pub id: String,
    pub class: SubproblemClass,
    pub depot_id: String,
    pub delivery_ids: Vec<String>,
    pub vehicle_ids: Vec<String>,
}

impl Subproblem {
    /// Binary variables an assignment formulation of this piece would need.
    pub fn variable_estimate(&self) -> usize {
        self.delivery_ids.len() * self.vehicle_ids.len() + self.vehicle_ids.len()
    }
}

/// A way of splitting an instance.
pub trait DecompositionStrategy: Send + Sync {
    fn name(&self) -> &str;
    fn rationale(&self) -> String;
    fn decompose(&self, problem: &DeliveryProblem, network: &Network) -> Result<Vec<Subproblem>>;
}

/// Solves the instance in one piece. Correct for small problems, hopeless for
/// large ones.
#[derive(Debug, Default, Clone)]
pub struct SingleBlock;

impl DecompositionStrategy for SingleBlock {
    fn name(&self) -> &str {
        "single-block"
    }

    fn rationale(&self) -> String {
        "the instance is small enough to formulate as one assignment problem".into()
    }

    fn decompose(&self, problem: &DeliveryProblem, _network: &Network) -> Result<Vec<Subproblem>> {
        let depot = primary_depot(problem)?;
        Ok(vec![Subproblem {
            id: format!("{}-single", problem.id),
            class: SubproblemClass::VehicleAssignment,
            depot_id: depot.clone(),
            delivery_ids: problem
                .deliveries
                .iter()
                .map(|delivery| delivery.id.clone())
                .collect(),
            vehicle_ids: problem
                .vehicles
                .iter()
                .map(|vehicle| vehicle.id.clone())
                .collect(),
        }])
    }
}

/// One subproblem per depot: the natural split for multi-depot networks.
#[derive(Debug, Default, Clone)]
pub struct DepotPartition;

impl DecompositionStrategy for DepotPartition {
    fn name(&self) -> &str {
        "depot-partition"
    }

    fn rationale(&self) -> String {
        "each depot serves its own deliveries with its own vehicles".into()
    }

    fn decompose(&self, problem: &DeliveryProblem, network: &Network) -> Result<Vec<Subproblem>> {
        if problem.depots.is_empty() {
            return Err(QuantumClawError::new(
                "depot partitioning needs at least one depot",
            ));
        }

        let mut buckets: BTreeMap<String, Vec<String>> = problem
            .depots
            .iter()
            .map(|depot| (depot.id.clone(), Vec::new()))
            .collect();

        for delivery in &problem.deliveries {
            // Honour an explicit depot, otherwise use the nearest one.
            let depot = match &delivery.depot_id {
                Some(depot_id) => depot_id.clone(),
                None => problem
                    .depots
                    .iter()
                    .map(|depot| {
                        (
                            depot.id.clone(),
                            network.distance_km(&depot.id, &delivery.id),
                        )
                    })
                    .min_by(|left, right| left.1.total_cmp(&right.1))
                    .map(|(id, _)| id)
                    .expect("at least one depot exists"),
            };
            buckets.entry(depot).or_default().push(delivery.id.clone());
        }

        Ok(buckets
            .into_iter()
            .filter(|(_, deliveries)| !deliveries.is_empty())
            .map(|(depot_id, delivery_ids)| Subproblem {
                id: format!("{}-{depot_id}", problem.id),
                class: SubproblemClass::VehicleAssignment,
                vehicle_ids: problem
                    .vehicles_at(&depot_id)
                    .into_iter()
                    .map(|vehicle| vehicle.id.clone())
                    .collect(),
                depot_id,
                delivery_ids,
            })
            .collect())
    }
}

/// Splits a depot's deliveries into geographic clusters of bounded size.
///
/// Uses a deterministic sweep around the depot: stops are ordered by bearing
/// and cut into blocks. The result does not depend on random seeding, which
/// keeps benchmarks reproducible.
#[derive(Debug, Clone)]
pub struct GeographicCluster {
    pub max_deliveries_per_cluster: usize,
}

impl Default for GeographicCluster {
    fn default() -> Self {
        Self {
            max_deliveries_per_cluster: 12,
        }
    }
}

impl DecompositionStrategy for GeographicCluster {
    fn name(&self) -> &str {
        "geographic-cluster"
    }

    fn rationale(&self) -> String {
        format!(
            "deliveries are swept into geographic clusters of at most {} stops",
            self.max_deliveries_per_cluster
        )
    }

    fn decompose(&self, problem: &DeliveryProblem, network: &Network) -> Result<Vec<Subproblem>> {
        let mut clusters = Vec::new();
        for parent in DepotPartition.decompose(problem, network)? {
            let depot = problem.depot(&parent.depot_id).ok_or_else(|| {
                QuantumClawError::new(format!("unknown depot '{}'", parent.depot_id))
            })?;

            let mut ordered: Vec<(f64, String)> = parent
                .delivery_ids
                .iter()
                .filter_map(|id| problem.delivery(id))
                .map(|delivery| {
                    let bearing = (delivery.location.lat - depot.location.lat)
                        .atan2(delivery.location.lon - depot.location.lon);
                    (bearing, delivery.id.clone())
                })
                .collect();
            ordered.sort_by(|left, right| {
                left.0
                    .total_cmp(&right.0)
                    .then_with(|| left.1.cmp(&right.1))
            });

            let chunk_size = self.max_deliveries_per_cluster.max(1);
            for (index, chunk) in ordered.chunks(chunk_size).enumerate() {
                clusters.push(Subproblem {
                    id: format!("{}-cluster-{index}", parent.id),
                    class: SubproblemClass::VehicleAssignment,
                    depot_id: parent.depot_id.clone(),
                    delivery_ids: chunk.iter().map(|(_, id)| id.clone()).collect(),
                    vehicle_ids: parent.vehicle_ids.clone(),
                });
            }
        }
        Ok(clusters)
    }
}

/// Splits deliveries so each block's demand fits a bounded number of vehicles.
#[derive(Debug, Clone)]
pub struct CapacityCluster {
    pub vehicles_per_block: usize,
}

impl Default for CapacityCluster {
    fn default() -> Self {
        Self {
            vehicles_per_block: 3,
        }
    }
}

impl DecompositionStrategy for CapacityCluster {
    fn name(&self) -> &str {
        "capacity-cluster"
    }

    fn rationale(&self) -> String {
        format!(
            "deliveries are grouped so each block loads at most {} vehicles",
            self.vehicles_per_block
        )
    }

    fn decompose(&self, problem: &DeliveryProblem, network: &Network) -> Result<Vec<Subproblem>> {
        let mut blocks = Vec::new();
        for parent in DepotPartition.decompose(problem, network)? {
            let vehicles: Vec<&crate::quantumclaw_brains_router::models::Vehicle> = parent
                .vehicle_ids
                .iter()
                .filter_map(|id| problem.vehicle(id))
                .collect();
            if vehicles.is_empty() {
                blocks.push(parent);
                continue;
            }

            let block_vehicles = self.vehicles_per_block.max(1).min(vehicles.len());
            let block_capacity: u32 = vehicles
                .iter()
                .take(block_vehicles)
                .map(|vehicle| vehicle.capacity)
                .sum();

            let mut current: Vec<String> = Vec::new();
            let mut current_demand = 0u32;
            let mut index = 0;
            for delivery_id in &parent.delivery_ids {
                let demand = problem
                    .delivery(delivery_id)
                    .map(|delivery| delivery.demand)
                    .unwrap_or(0);
                if !current.is_empty() && current_demand + demand > block_capacity {
                    blocks.push(block(&parent, index, current.clone(), block_vehicles));
                    index += 1;
                    current.clear();
                    current_demand = 0;
                }
                current.push(delivery_id.clone());
                current_demand += demand;
            }
            if !current.is_empty() {
                blocks.push(block(&parent, index, current, block_vehicles));
            }
        }
        Ok(blocks)
    }
}

fn block(
    parent: &Subproblem,
    index: usize,
    delivery_ids: Vec<String>,
    block_vehicles: usize,
) -> Subproblem {
    // Vehicles are shared across blocks by rotation so no block monopolizes the
    // fleet; the decoder re-checks capacity across the reassembled plan.
    let offset = index * block_vehicles;
    let vehicle_ids: Vec<String> = (0..block_vehicles)
        .map(|position| parent.vehicle_ids[(offset + position) % parent.vehicle_ids.len()].clone())
        .collect();
    Subproblem {
        id: format!("{}-capacity-{index}", parent.id),
        class: SubproblemClass::VehicleAssignment,
        depot_id: parent.depot_id.clone(),
        delivery_ids,
        vehicle_ids,
    }
}

/// Splits deliveries by the time window they must be served in.
#[derive(Debug, Clone)]
pub struct TimeWindowPartition {
    pub bucket_minutes: f64,
}

impl Default for TimeWindowPartition {
    fn default() -> Self {
        Self {
            bucket_minutes: 240.0,
        }
    }
}

impl DecompositionStrategy for TimeWindowPartition {
    fn name(&self) -> &str {
        "time-window-partition"
    }

    fn rationale(&self) -> String {
        format!(
            "deliveries are grouped into {}-minute service buckets",
            self.bucket_minutes
        )
    }

    fn decompose(&self, problem: &DeliveryProblem, network: &Network) -> Result<Vec<Subproblem>> {
        let bucket_minutes = if self.bucket_minutes > 0.0 {
            self.bucket_minutes
        } else {
            240.0
        };
        let mut blocks = Vec::new();
        for parent in DepotPartition.decompose(problem, network)? {
            let mut buckets: BTreeMap<i64, Vec<String>> = BTreeMap::new();
            for delivery_id in &parent.delivery_ids {
                let bucket = problem
                    .delivery(delivery_id)
                    .and_then(|delivery| delivery.window)
                    .map(|window| (window.start_min / bucket_minutes).floor() as i64)
                    .unwrap_or(i64::MIN);
                buckets.entry(bucket).or_default().push(delivery_id.clone());
            }
            for (index, (_, delivery_ids)) in buckets.into_iter().enumerate() {
                blocks.push(Subproblem {
                    id: format!("{}-window-{index}", parent.id),
                    class: SubproblemClass::VehicleAssignment,
                    depot_id: parent.depot_id.clone(),
                    delivery_ids,
                    vehicle_ids: parent.vehicle_ids.clone(),
                });
            }
        }
        Ok(blocks)
    }
}

/// Splits a long horizon into consecutive slices solved in order.
#[derive(Debug, Clone)]
pub struct RollingHorizon {
    pub horizon_minutes: f64,
}

impl Default for RollingHorizon {
    fn default() -> Self {
        Self {
            horizon_minutes: 480.0,
        }
    }
}

impl DecompositionStrategy for RollingHorizon {
    fn name(&self) -> &str {
        "rolling-horizon"
    }

    fn rationale(&self) -> String {
        format!(
            "the day is optimized in consecutive {}-minute horizons",
            self.horizon_minutes
        )
    }

    fn decompose(&self, problem: &DeliveryProblem, network: &Network) -> Result<Vec<Subproblem>> {
        TimeWindowPartition {
            bucket_minutes: self.horizon_minutes,
        }
        .decompose(problem, network)
        .map(|blocks| {
            blocks
                .into_iter()
                .enumerate()
                .map(|(index, mut block)| {
                    block.id = format!("{}-horizon-{index}", problem.id);
                    block
                })
                .collect()
        })
    }
}

/// Picks a strategy from the shape of the instance.
///
/// The rule is deliberately simple and inspectable: split by depot, then keep
/// splitting until each piece fits the configured variable budget.
#[derive(Debug, Clone)]
pub struct DecompositionPolicy {
    /// Largest binary model a single subproblem may produce.
    pub max_variables_per_subproblem: usize,
}

impl Default for DecompositionPolicy {
    fn default() -> Self {
        Self {
            max_variables_per_subproblem: 60,
        }
    }
}

impl DecompositionPolicy {
    /// Decomposes an instance, escalating strategies until the pieces fit.
    pub fn decompose(
        &self,
        problem: &DeliveryProblem,
        network: &Network,
    ) -> Result<(Vec<Subproblem>, Decomposition)> {
        let strategies: Vec<Box<dyn DecompositionStrategy>> = vec![
            Box::new(SingleBlock),
            Box::new(DepotPartition),
            Box::new(GeographicCluster::default()),
            Box::new(GeographicCluster {
                max_deliveries_per_cluster: 6,
            }),
            Box::new(GeographicCluster {
                max_deliveries_per_cluster: 3,
            }),
        ];

        let mut last: Option<(Vec<Subproblem>, String, String)> = None;
        for strategy in strategies {
            let subproblems = strategy.decompose(problem, network)?;
            let fits = subproblems
                .iter()
                .all(|piece| piece.variable_estimate() <= self.max_variables_per_subproblem);
            last = Some((
                subproblems,
                strategy.name().to_string(),
                strategy.rationale(),
            ));
            if fits {
                break;
            }
        }

        let (subproblems, name, rationale) = last.ok_or_else(|| {
            QuantumClawError::new("no decomposition strategy produced any subproblem")
        })?;
        let summary = Decomposition {
            strategy: name,
            rationale,
            subproblems: subproblems
                .iter()
                .map(|piece| SubproblemSummary {
                    id: piece.id.clone(),
                    class: piece.class.as_str().to_string(),
                    size: piece.variable_estimate(),
                    members: piece.delivery_ids.clone(),
                })
                .collect(),
        };
        Ok((subproblems, summary))
    }
}

/// Named strategy lookup, used by callers that want to force one.
pub fn strategy_by_name(name: &str) -> Option<Box<dyn DecompositionStrategy>> {
    match name {
        "single-block" => Some(Box::new(SingleBlock)),
        "depot-partition" => Some(Box::new(DepotPartition)),
        "geographic-cluster" => Some(Box::new(GeographicCluster::default())),
        "capacity-cluster" => Some(Box::new(CapacityCluster::default())),
        "time-window-partition" => Some(Box::new(TimeWindowPartition::default())),
        "rolling-horizon" => Some(Box::new(RollingHorizon::default())),
        _ => None,
    }
}

/// Every strategy name this brain understands.
pub fn strategy_names() -> Vec<&'static str> {
    vec![
        "single-block",
        "depot-partition",
        "geographic-cluster",
        "capacity-cluster",
        "time-window-partition",
        "rolling-horizon",
    ]
}

fn primary_depot(problem: &DeliveryProblem) -> Result<String> {
    problem
        .depots
        .first()
        .map(|depot| depot.id.clone())
        .ok_or_else(|| QuantumClawError::new("the problem declares no depot"))
}

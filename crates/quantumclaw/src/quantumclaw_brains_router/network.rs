//! Travel distances and times between nodes.

use crate::quantumclaw_brains_router::models::{DeliveryProblem, DistanceMatrix};
use crate::quantumclaw_core::QuantumClawError;
use std::collections::BTreeMap;

/// Distances and durations for one problem instance.
///
/// Built once per solve so no lookup repeats a haversine computation or a
/// linear scan of the delivery list.
#[derive(Debug, Clone)]
pub struct Network {
    index: BTreeMap<String, usize>,
    distances_km: Vec<Vec<f64>>,
    durations_min: Vec<Vec<f64>>,
}

impl Network {
    /// Builds the network, failing when an explicit matrix does not line up
    /// with the nodes it is supposed to describe.
    pub fn build(problem: &DeliveryProblem) -> Result<Self, QuantumClawError> {
        match &problem.matrix {
            DistanceMatrix::Haversine { average_speed_kmh } => {
                Self::from_coordinates(problem, *average_speed_kmh)
            }
            DistanceMatrix::Explicit {
                nodes,
                distances_km,
                durations_min,
            } => Self::from_explicit(nodes, distances_km, durations_min.as_ref()),
        }
    }

    fn from_coordinates(
        problem: &DeliveryProblem,
        average_speed_kmh: f64,
    ) -> Result<Self, QuantumClawError> {
        if average_speed_kmh <= 0.0 {
            return Err(QuantumClawError::new(
                "the distance matrix needs a positive average speed",
            ));
        }

        let mut index = BTreeMap::new();
        let mut locations = Vec::new();
        for depot in &problem.depots {
            index.insert(depot.id.clone(), locations.len());
            locations.push(depot.location);
        }
        for delivery in &problem.deliveries {
            index.insert(delivery.id.clone(), locations.len());
            locations.push(delivery.location);
        }

        let size = locations.len();
        let mut distances_km = vec![vec![0.0; size]; size];
        let mut durations_min = vec![vec![0.0; size]; size];
        for (row, origin) in locations.iter().enumerate() {
            for (column, destination) in locations.iter().enumerate() {
                let distance = origin.haversine_km(destination);
                distances_km[row][column] = distance;
                durations_min[row][column] = distance / average_speed_kmh * 60.0;
            }
        }

        Ok(Self {
            index,
            distances_km,
            durations_min,
        })
    }

    fn from_explicit(
        nodes: &[String],
        distances_km: &[Vec<f64>],
        durations_min: Option<&Vec<Vec<f64>>>,
    ) -> Result<Self, QuantumClawError> {
        let size = nodes.len();
        if distances_km.len() != size || distances_km.iter().any(|row| row.len() != size) {
            return Err(QuantumClawError::new(format!(
                "the explicit distance matrix must be {size}x{size} to match its {size} nodes"
            )));
        }
        if let Some(durations) = durations_min {
            if durations.len() != size || durations.iter().any(|row| row.len() != size) {
                return Err(QuantumClawError::new(format!(
                    "the explicit duration matrix must be {size}x{size} to match its {size} nodes"
                )));
            }
        }

        let index = nodes
            .iter()
            .enumerate()
            .map(|(position, id)| (id.clone(), position))
            .collect();
        // Without explicit durations, travel time is derived from distance at a
        // nominal 40 km/h so time windows still mean something.
        let durations = durations_min.cloned().unwrap_or_else(|| {
            distances_km
                .iter()
                .map(|row| row.iter().map(|km| km / 40.0 * 60.0).collect())
                .collect()
        });

        Ok(Self {
            index,
            distances_km: distances_km.to_vec(),
            durations_min: durations,
        })
    }

    fn position(&self, node: &str) -> Option<usize> {
        self.index.get(node).copied()
    }

    /// Distance in kilometres. Unknown nodes contribute nothing rather than
    /// panicking; validation is responsible for rejecting them up front.
    pub fn distance_km(&self, from: &str, to: &str) -> f64 {
        match (self.position(from), self.position(to)) {
            (Some(origin), Some(destination)) => self.distances_km[origin][destination],
            _ => 0.0,
        }
    }

    pub fn duration_min(&self, from: &str, to: &str) -> f64 {
        match (self.position(from), self.position(to)) {
            (Some(origin), Some(destination)) => self.durations_min[origin][destination],
            _ => 0.0,
        }
    }

    pub fn knows(&self, node: &str) -> bool {
        self.index.contains_key(node)
    }

    /// Length of a closed tour that leaves the depot, visits stops in order,
    /// and returns.
    pub fn route_distance_km(&self, depot: &str, stops: &[String]) -> f64 {
        if stops.is_empty() {
            return 0.0;
        }
        let mut total = self.distance_km(depot, &stops[0]);
        for pair in stops.windows(2) {
            total += self.distance_km(&pair[0], &pair[1]);
        }
        total + self.distance_km(&stops[stops.len() - 1], depot)
    }

    /// Driving time of the same closed tour, excluding service time.
    pub fn route_duration_min(&self, depot: &str, stops: &[String]) -> f64 {
        if stops.is_empty() {
            return 0.0;
        }
        let mut total = self.duration_min(depot, &stops[0]);
        for pair in stops.windows(2) {
            total += self.duration_min(&pair[0], &pair[1]);
        }
        total + self.duration_min(&stops[stops.len() - 1], depot)
    }
}

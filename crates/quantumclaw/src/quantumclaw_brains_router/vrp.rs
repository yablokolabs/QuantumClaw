//! Classical route construction and improvement.
//!
//! Sequencing a handful of stops is something classical heuristics do well and
//! cheaply. Q-Router keeps it classical on purpose and spends its combinatorial
//! budget on assignment instead, where the search space actually justifies a
//! QUBO.

use crate::quantumclaw_brains_router::network::Network;

/// Orders stops by repeatedly hopping to the nearest unvisited one.
pub fn nearest_neighbor(network: &Network, depot: &str, stops: &[String]) -> Vec<String> {
    let mut remaining: Vec<String> = stops.to_vec();
    let mut ordered = Vec::with_capacity(remaining.len());
    let mut current = depot.to_string();

    while !remaining.is_empty() {
        let (index, _) = remaining
            .iter()
            .enumerate()
            .map(|(index, candidate)| (index, network.distance_km(&current, candidate)))
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .expect("remaining is not empty");
        current = remaining.remove(index);
        ordered.push(current.clone());
    }

    ordered
}

/// Removes crossings by reversing segments while that shortens the tour.
pub fn two_opt(network: &Network, depot: &str, stops: &[String]) -> Vec<String> {
    let mut best: Vec<String> = stops.to_vec();
    if best.len() < 3 {
        return best;
    }

    let mut best_distance = network.route_distance_km(depot, &best);
    let mut improved = true;
    // Bounded so a pathological instance cannot spin here.
    let max_passes = 50;
    let mut passes = 0;

    while improved && passes < max_passes {
        improved = false;
        passes += 1;
        for first in 0..best.len() - 1 {
            for second in first + 1..best.len() {
                let mut candidate = best.clone();
                candidate[first..=second].reverse();
                let distance = network.route_distance_km(depot, &candidate);
                if distance + 1e-9 < best_distance {
                    best = candidate;
                    best_distance = distance;
                    improved = true;
                }
            }
        }
    }

    best
}

/// Moves single stops to better positions, including across the tour.
pub fn or_opt(network: &Network, depot: &str, stops: &[String]) -> Vec<String> {
    let mut best = stops.to_vec();
    if best.len() < 3 {
        return best;
    }
    let mut best_distance = network.route_distance_km(depot, &best);

    for origin in 0..best.len() {
        for target in 0..best.len() {
            if origin == target {
                continue;
            }
            let mut candidate = best.clone();
            let moved = candidate.remove(origin);
            candidate.insert(target.min(candidate.len()), moved);
            let distance = network.route_distance_km(depot, &candidate);
            if distance + 1e-9 < best_distance {
                best = candidate;
                best_distance = distance;
            }
        }
    }

    best
}

/// Builds a good visiting order: nearest neighbour, then local improvement.
pub fn sequence(network: &Network, depot: &str, stops: &[String]) -> Vec<String> {
    if stops.len() < 2 {
        return stops.to_vec();
    }
    let constructed = nearest_neighbor(network, depot, stops);
    let improved = two_opt(network, depot, &constructed);
    or_opt(network, depot, &improved)
}

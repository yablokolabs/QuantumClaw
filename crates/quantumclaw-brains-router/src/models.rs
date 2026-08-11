//! Logistics domain model.
//!
//! Everything here is vocabulary a dispatcher would recognize: depots,
//! vehicles, deliveries, time windows, distance matrices. None of it knows
//! about binary variables, QUBOs, or solvers.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A geographic point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
}

impl Location {
    pub fn new(lat: f64, lon: f64) -> Self {
        Self { lat, lon }
    }

    /// Great-circle distance in kilometres.
    pub fn haversine_km(&self, other: &Self) -> f64 {
        const EARTH_RADIUS_KM: f64 = 6371.0088;
        let (lat1, lat2) = (self.lat.to_radians(), other.lat.to_radians());
        let delta_lat = (other.lat - self.lat).to_radians();
        let delta_lon = (other.lon - self.lon).to_radians();
        let a = (delta_lat / 2.0).sin().powi(2)
            + lat1.cos() * lat2.cos() * (delta_lon / 2.0).sin().powi(2);
        2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
    }
}

/// Minutes from the start of the planning horizon.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimeWindow {
    pub start_min: f64,
    pub end_min: f64,
}

impl TimeWindow {
    pub fn new(start_min: f64, end_min: f64) -> Self {
        Self { start_min, end_min }
    }

    pub fn contains(&self, minute: f64) -> bool {
        minute >= self.start_min && minute <= self.end_min
    }

    /// Minutes by which an arrival misses the window's close.
    pub fn lateness(&self, arrival_min: f64) -> f64 {
        (arrival_min - self.end_min).max(0.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Depot {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub location: Location,
}

impl Depot {
    pub fn new(id: impl Into<String>, location: Location) -> Self {
        let id = id.into();
        Self {
            name: id.clone(),
            id,
            location,
        }
    }
}

/// A vehicle in a heterogeneous fleet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vehicle {
    pub id: String,
    pub depot_id: String,
    /// Capacity in whatever integer unit the deliveries use.
    pub capacity: u32,
    #[serde(default = "default_cost_per_km")]
    pub cost_per_km: f64,
    #[serde(default)]
    pub fixed_cost: f64,
    #[serde(default = "default_fuel")]
    pub fuel_l_per_100km: f64,
    #[serde(default = "default_co2")]
    pub co2_g_per_km: f64,
    #[serde(default = "default_speed")]
    pub average_speed_kmh: f64,
    #[serde(default)]
    pub max_distance_km: Option<f64>,
    /// Driver shift, used to detect over-long routes.
    #[serde(default)]
    pub shift: Option<TimeWindow>,
}

fn default_cost_per_km() -> f64 {
    1.0
}

fn default_fuel() -> f64 {
    28.0
}

fn default_co2() -> f64 {
    740.0
}

fn default_speed() -> f64 {
    40.0
}

impl Vehicle {
    pub fn new(id: impl Into<String>, depot_id: impl Into<String>, capacity: u32) -> Self {
        Self {
            id: id.into(),
            depot_id: depot_id.into(),
            capacity,
            cost_per_km: default_cost_per_km(),
            fixed_cost: 0.0,
            fuel_l_per_100km: default_fuel(),
            co2_g_per_km: default_co2(),
            average_speed_kmh: default_speed(),
            max_distance_km: None,
            shift: None,
        }
    }

    pub fn with_cost_per_km(mut self, cost_per_km: f64) -> Self {
        self.cost_per_km = cost_per_km;
        self
    }

    pub fn with_fixed_cost(mut self, fixed_cost: f64) -> Self {
        self.fixed_cost = fixed_cost;
        self
    }

    pub fn with_fuel_l_per_100km(mut self, fuel_l_per_100km: f64) -> Self {
        self.fuel_l_per_100km = fuel_l_per_100km;
        self
    }

    pub fn with_co2_g_per_km(mut self, co2_g_per_km: f64) -> Self {
        self.co2_g_per_km = co2_g_per_km;
        self
    }

    pub fn with_average_speed_kmh(mut self, average_speed_kmh: f64) -> Self {
        self.average_speed_kmh = average_speed_kmh;
        self
    }

    pub fn with_shift(mut self, shift: TimeWindow) -> Self {
        self.shift = Some(shift);
        self
    }
}

/// One stop to serve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    pub id: String,
    pub location: Location,
    pub demand: u32,
    #[serde(default)]
    pub service_time_min: f64,
    #[serde(default)]
    pub window: Option<TimeWindow>,
    /// Higher priority deliveries are preferred when not everything fits.
    #[serde(default = "default_priority")]
    pub priority: f64,
    /// Restricts the delivery to one depot when set.
    #[serde(default)]
    pub depot_id: Option<String>,
}

fn default_priority() -> f64 {
    1.0
}

impl Delivery {
    pub fn new(id: impl Into<String>, location: Location, demand: u32) -> Self {
        Self {
            id: id.into(),
            location,
            demand,
            service_time_min: 0.0,
            window: None,
            priority: default_priority(),
            depot_id: None,
        }
    }

    pub fn with_window(mut self, window: TimeWindow) -> Self {
        self.window = Some(window);
        self
    }

    pub fn with_service_time_min(mut self, service_time_min: f64) -> Self {
        self.service_time_min = service_time_min;
        self
    }

    pub fn with_depot(mut self, depot_id: impl Into<String>) -> Self {
        self.depot_id = Some(depot_id.into());
        self
    }
}

/// How travel between nodes is measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DistanceMatrix {
    /// Straight-line distance from coordinates, with a nominal speed.
    Haversine {
        #[serde(default = "default_speed")]
        average_speed_kmh: f64,
    },
    /// Explicit matrix indexed by node id, for real road distances.
    Explicit {
        nodes: Vec<String>,
        distances_km: Vec<Vec<f64>>,
        #[serde(default)]
        durations_min: Option<Vec<Vec<f64>>>,
    },
}

impl Default for DistanceMatrix {
    fn default() -> Self {
        Self::Haversine {
            average_speed_kmh: default_speed(),
        }
    }
}

/// Money and emissions assumptions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterCostModel {
    pub fuel_price_per_liter: f64,
    pub driver_cost_per_hour: f64,
}

impl Default for RouterCostModel {
    fn default() -> Self {
        Self {
            fuel_price_per_liter: 1.5,
            driver_cost_per_hour: 20.0,
        }
    }
}

/// Service-level commitments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlaPolicy {
    /// Cost charged per minute a delivery is late.
    pub late_penalty_per_minute: f64,
    /// Lateness above this counts as a breach rather than a delay.
    pub breach_after_minutes: f64,
}

impl Default for SlaPolicy {
    fn default() -> Self {
        Self {
            late_penalty_per_minute: 1.0,
            breach_after_minutes: 30.0,
        }
    }
}

/// A complete routing instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeliveryProblem {
    pub id: String,
    pub depots: Vec<Depot>,
    pub vehicles: Vec<Vehicle>,
    pub deliveries: Vec<Delivery>,
    #[serde(default)]
    pub matrix: DistanceMatrix,
    #[serde(default)]
    pub cost_model: RouterCostModel,
    #[serde(default)]
    pub sla: SlaPolicy,
    /// The customer's existing plan, used as the benchmark baseline.
    #[serde(default)]
    pub baseline: Option<RouteSolution>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl DeliveryProblem {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            depots: Vec::new(),
            vehicles: Vec::new(),
            deliveries: Vec::new(),
            matrix: DistanceMatrix::default(),
            cost_model: RouterCostModel::default(),
            sla: SlaPolicy::default(),
            baseline: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_depot(mut self, depot: Depot) -> Self {
        self.depots.push(depot);
        self
    }

    pub fn with_vehicle(mut self, vehicle: Vehicle) -> Self {
        self.vehicles.push(vehicle);
        self
    }

    pub fn with_delivery(mut self, delivery: Delivery) -> Self {
        self.deliveries.push(delivery);
        self
    }

    pub fn with_matrix(mut self, matrix: DistanceMatrix) -> Self {
        self.matrix = matrix;
        self
    }

    pub fn with_baseline(mut self, baseline: RouteSolution) -> Self {
        self.baseline = Some(baseline);
        self
    }

    pub fn delivery(&self, id: &str) -> Option<&Delivery> {
        self.deliveries.iter().find(|delivery| delivery.id == id)
    }

    pub fn vehicle(&self, id: &str) -> Option<&Vehicle> {
        self.vehicles.iter().find(|vehicle| vehicle.id == id)
    }

    pub fn depot(&self, id: &str) -> Option<&Depot> {
        self.depots.iter().find(|depot| depot.id == id)
    }

    pub fn total_demand(&self) -> u32 {
        self.deliveries.iter().map(|delivery| delivery.demand).sum()
    }

    pub fn total_capacity(&self) -> u32 {
        self.vehicles.iter().map(|vehicle| vehicle.capacity).sum()
    }

    /// Vehicles stationed at a depot.
    pub fn vehicles_at(&self, depot_id: &str) -> Vec<&Vehicle> {
        self.vehicles
            .iter()
            .filter(|vehicle| vehicle.depot_id == depot_id)
            .collect()
    }
}

/// One vehicle's ordered itinerary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub vehicle_id: String,
    pub depot_id: String,
    /// Delivery ids in visit order.
    pub stops: Vec<String>,
}

impl Route {
    pub fn new(vehicle_id: impl Into<String>, depot_id: impl Into<String>) -> Self {
        Self {
            vehicle_id: vehicle_id.into(),
            depot_id: depot_id.into(),
            stops: Vec::new(),
        }
    }

    pub fn with_stops<I, S>(mut self, stops: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.stops = stops.into_iter().map(Into::into).collect();
        self
    }

    pub fn is_empty(&self) -> bool {
        self.stops.is_empty()
    }
}

/// A complete plan: who visits what, and what could not be served.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteSolution {
    pub problem_id: String,
    pub routes: Vec<Route>,
    #[serde(default)]
    pub unassigned: Vec<String>,
}

impl RouteSolution {
    pub fn new(problem_id: impl Into<String>) -> Self {
        Self {
            problem_id: problem_id.into(),
            routes: Vec::new(),
            unassigned: Vec::new(),
        }
    }

    pub fn with_route(mut self, route: Route) -> Self {
        self.routes.push(route);
        self
    }

    pub fn served_deliveries(&self) -> Vec<&String> {
        self.routes
            .iter()
            .flat_map(|route| route.stops.iter())
            .collect()
    }

    pub fn vehicles_used(&self) -> usize {
        self.routes.iter().filter(|route| !route.is_empty()).count()
    }
}

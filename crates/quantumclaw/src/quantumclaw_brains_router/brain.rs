//! The Q-Router brain.
//!
//! ```text
//! delivery problem
//!   -> validate
//!   -> decompose
//!   -> formulate each piece as an OptimizationProblem
//!   -> SolverBackend (classical / dwave-sa / dwave-hybrid / dwave-qpu)
//!   -> decode assignments, repair, sequence classically
//!   -> re-check constraints
//!   -> logistics KPIs
//! ```
//!
//! The brain never names a provider in its logic. It asks a
//! [`crate::quantumclaw_core::SolverRegistry`] for a backend by name and treats every
//! backend identically.

use crate::quantumclaw_brains::{
    BrainCapabilities, BrainMatch, BrainPlan, BrainSolveContext, Decomposition, Explanation,
    Formulation, KpiReport, QuantumBrain, ValidationReport,
};
use crate::quantumclaw_brains_router::compiler::{assignment_problem, AssignmentWeights};
use crate::quantumclaw_brains_router::constraints::{solution_violations, RouterViolation};
use crate::quantumclaw_brains_router::decoder::{
    assignments_from_solution, build_solution, greedy_assignment, repair, Assignment,
};
use crate::quantumclaw_brains_router::decomposition::{
    strategy_by_name, strategy_names, DecompositionPolicy, Subproblem, SubproblemClass,
};
use crate::quantumclaw_brains_router::kpis::{self, RouterKpis};
use crate::quantumclaw_brains_router::models::{DeliveryProblem, RouteSolution};
use crate::quantumclaw_brains_router::network::Network;
use crate::quantumclaw_brains_router::routing_policy::{LedgerRecord, SolverRoutingPolicy};
use crate::quantumclaw_brains_router::sequencing::{
    decode_sequence, tsp_problem, SequencingChoice, SequencingPolicy, SequencingReport,
};
use crate::quantumclaw_core::{
    AgentTask, QuantumClawError, Result, SolverBackend, SolverContext, SolverKind,
};
use crate::quantumclaw_ir::DecisionProblem;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

/// Backend name that forces the brain's own classical path.
///
/// Without this, "no backend requested" means "let the routing policy decide",
/// which will happily pick a sampler — so a benchmark row labelled classical
/// would not be classical at all.
pub const BACKEND_CLASSICAL: &str = "classical";

/// Vocabulary that marks a task as belonging to this brain.
const DOMAIN_KEYWORDS: &[&str] = &[
    "delivery",
    "deliveries",
    "route",
    "routing",
    "fleet",
    "vehicle",
    "truck",
    "depot",
    "dispatch",
    "logistics",
    "vrp",
];

/// Knobs a caller can turn per request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterOptions {
    /// Force one backend by name. When set and the backend fails, the failure
    /// is reported rather than quietly replaced by a classical result.
    #[serde(default)]
    pub backend: Option<String>,
    /// Force one decomposition strategy by name.
    #[serde(default)]
    pub decomposition: Option<String>,
    /// Largest binary model a subproblem may produce before it is split again.
    #[serde(default = "default_max_variables")]
    pub max_variables_per_subproblem: usize,
    /// Whether route sequencing is also offered to a sampler. Off by default.
    #[serde(default)]
    pub sequencing: SequencingPolicy,
    /// Seed handed to stochastic samplers, making a run reproducible.
    #[serde(default)]
    pub sampler_seed: Option<u64>,
}

fn default_max_variables() -> usize {
    60
}

impl Default for RouterOptions {
    fn default() -> Self {
        Self {
            backend: None,
            decomposition: None,
            max_variables_per_subproblem: default_max_variables(),
            sequencing: SequencingPolicy::default(),
            sampler_seed: None,
        }
    }
}

/// What the brain was asked to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QRouterRequest {
    pub problem: DeliveryProblem,
    #[serde(default)]
    pub options: RouterOptions,
}

impl QRouterRequest {
    pub fn new(problem: DeliveryProblem) -> Self {
        Self {
            problem,
            options: RouterOptions::default(),
        }
    }

    pub fn with_backend(mut self, backend: impl Into<String>) -> Self {
        self.options.backend = Some(backend.into());
        self
    }

    pub fn with_decomposition(mut self, strategy: impl Into<String>) -> Self {
        self.options.decomposition = Some(strategy.into());
        self
    }

    pub fn with_sampler_seed(mut self, seed: u64) -> Self {
        self.options.sampler_seed = Some(seed);
        self
    }
}

/// How one subproblem was solved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubproblemReport {
    pub id: String,
    pub class: String,
    pub deliveries: usize,
    pub vehicles: usize,
    pub variables: usize,
    /// Backend that produced the assignment, or `classical-greedy`.
    pub backend: String,
    pub backend_kind: Option<SolverKind>,
    pub routing_reason: String,
    pub objective: Option<f64>,
    pub feasible: bool,
    pub runtime_ms: u64,
    pub solver_runtime_ms: Option<f64>,
    /// Why the brain fell back, when it did.
    #[serde(default)]
    pub fallback_reason: Option<String>,
}

/// The brain's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QRouterResult {
    pub problem_id: String,
    pub problem: DeliveryProblem,
    pub solution: RouteSolution,
    pub kpis: RouterKpis,
    pub decomposition: Decomposition,
    pub subproblems: Vec<SubproblemReport>,
    /// One entry per route considered for QUBO sequencing. Empty when the
    /// sequencing lane is off, which is the default.
    #[serde(default)]
    pub sequencing: Vec<SequencingReport>,
    pub violations: Vec<RouterViolation>,
    pub feasible: bool,
    pub runtime_ms: u64,
}

/// Logistics optimization brain.
#[derive(Debug, Clone, Default)]
pub struct QRouterBrain {
    pub weights: AssignmentWeights,
    pub routing_policy: SolverRoutingPolicy,
}

impl QRouterBrain {
    pub fn new() -> Self {
        Self {
            weights: AssignmentWeights::default(),
            routing_policy: SolverRoutingPolicy::default(),
        }
    }

    pub fn with_routing_policy(mut self, routing_policy: SolverRoutingPolicy) -> Self {
        self.routing_policy = routing_policy;
        self
    }

    /// Splits an instance according to the request's options.
    pub fn decompose_problem(
        &self,
        request: &QRouterRequest,
        network: &Network,
    ) -> Result<(Vec<Subproblem>, Decomposition)> {
        match &request.options.decomposition {
            Some(name) => {
                let strategy = strategy_by_name(name).ok_or_else(|| {
                    QuantumClawError::new(format!(
                        "unknown decomposition strategy '{name}'; available strategies: {}",
                        strategy_names().join(", ")
                    ))
                })?;
                let subproblems = strategy.decompose(&request.problem, network)?;
                let summary = Decomposition {
                    strategy: strategy.name().to_string(),
                    rationale: strategy.rationale(),
                    subproblems: subproblems
                        .iter()
                        .map(|piece| crate::quantumclaw_brains::SubproblemSummary {
                            id: piece.id.clone(),
                            class: piece.class.as_str().to_string(),
                            size: piece.variable_estimate(),
                            members: piece.delivery_ids.clone(),
                        })
                        .collect(),
                };
                Ok((subproblems, summary))
            }
            None => DecompositionPolicy {
                max_variables_per_subproblem: request.options.max_variables_per_subproblem,
            }
            .decompose(&request.problem, network),
        }
    }

    /// Solves one subproblem, returning the assignment and how it was produced.
    async fn solve_subproblem(
        &self,
        request: &QRouterRequest,
        network: &Network,
        subproblem: &Subproblem,
        context: &BrainSolveContext,
    ) -> Result<(Assignment, SubproblemReport)> {
        let started = Instant::now();
        let model = assignment_problem(&request.problem, network, subproblem, &self.weights)?;
        let variables = model.variables.len();
        let available = context.registry.names();

        let explicit = request
            .options
            .backend
            .clone()
            .or_else(|| context.requested_backend.clone());
        let decision = match explicit.as_deref() {
            Some(BACKEND_CLASSICAL) => {
                crate::quantumclaw_brains_router::routing_policy::RoutingDecision {
                    backend: None,
                    reason: "the caller asked for the classical path explicitly".into(),
                }
            }
            Some(name) => crate::quantumclaw_brains_router::routing_policy::RoutingDecision {
                backend: Some(name.to_string()),
                reason: format!("the caller requested '{name}'"),
            },
            None => self
                .routing_policy
                .choose(subproblem.class.as_str(), variables, &available),
        };

        let mut report = SubproblemReport {
            id: subproblem.id.clone(),
            class: subproblem.class.as_str().to_string(),
            deliveries: subproblem.delivery_ids.len(),
            vehicles: subproblem.vehicle_ids.len(),
            variables,
            backend: "classical-greedy".into(),
            backend_kind: None,
            routing_reason: decision.reason.clone(),
            objective: None,
            feasible: true,
            runtime_ms: 0,
            solver_runtime_ms: None,
            fallback_reason: None,
        };

        let Some(backend_name) = decision.backend else {
            let assignment = greedy_assignment(
                &request.problem,
                network,
                &subproblem.delivery_ids,
                &subproblem.vehicle_ids,
            );
            report.runtime_ms = started.elapsed().as_millis() as u64;
            return Ok((assignment, report));
        };

        let backend: Arc<dyn SolverBackend> = match context.registry.require(&backend_name) {
            Ok(backend) => backend,
            // An explicit request that cannot be honoured is an error; an
            // automatic choice degrades to the classical path.
            Err(error) if explicit.is_some() => return Err(error),
            Err(error) => {
                report.fallback_reason = Some(error.to_string());
                let assignment = greedy_assignment(
                    &request.problem,
                    network,
                    &subproblem.delivery_ids,
                    &subproblem.vehicle_ids,
                );
                report.runtime_ms = started.elapsed().as_millis() as u64;
                return Ok((assignment, report));
            }
        };

        let decision_problem = seeded_problem(
            DecisionProblem::new(subproblem.id.clone()).with_optimization(model.clone()),
            request.options.sampler_seed,
        );
        let solver_context = SolverContext::from_task(
            context
                .task
                .as_ref()
                .unwrap_or(&AgentTask::new("optimize vehicle assignment")),
        );

        match backend.solve(decision_problem, solver_context).await {
            Ok(output) => {
                report.backend = output.backend.clone();
                report.backend_kind = Some(output.backend_kind);
                report.solver_runtime_ms = output
                    .telemetry
                    .provider_metadata
                    .as_ref()
                    .and_then(|value| value.get("solver_runtime_ms"))
                    .and_then(|value| value.as_f64());

                let assignment = match output.solution {
                    Some(solution) => {
                        report.objective = Some(solution.objective_value);
                        report.feasible = solution.feasible;
                        assignments_from_solution(&solution, &model)
                    }
                    None => {
                        report.fallback_reason = Some(format!(
                            "backend '{backend_name}' returned no optimization result"
                        ));
                        greedy_assignment(
                            &request.problem,
                            network,
                            &subproblem.delivery_ids,
                            &subproblem.vehicle_ids,
                        )
                    }
                };
                report.runtime_ms = started.elapsed().as_millis() as u64;
                Ok((assignment, report))
            }
            Err(error) if explicit.is_some() => Err(QuantumClawError::new(format!(
                "requested backend '{backend_name}' failed: {error}"
            ))),
            Err(error) => {
                report.fallback_reason = Some(format!("backend '{backend_name}' failed: {error}"));
                let assignment = greedy_assignment(
                    &request.problem,
                    network,
                    &subproblem.delivery_ids,
                    &subproblem.vehicle_ids,
                );
                report.runtime_ms = started.elapsed().as_millis() as u64;
                Ok((assignment, report))
            }
        }
    }

    /// Re-sequences routes with a sampler, keeping whichever tour is shorter.
    ///
    /// A sampler is free to return a broken or worse tour; neither can reach
    /// the shipped plan. The classical route is the floor, always.
    async fn sequence_with_sampler(
        &self,
        request: &QRouterRequest,
        network: &Network,
        solution: &mut RouteSolution,
        context: &BrainSolveContext,
    ) -> Vec<SequencingReport> {
        let policy = &request.options.sequencing;
        let mut reports = Vec::new();
        if !policy.enabled {
            return reports;
        }

        let backend_name = policy
            .backend
            .clone()
            .or_else(|| request.options.backend.clone())
            .or_else(|| context.requested_backend.clone())
            .or_else(|| self.routing_policy.preferred_backends.first().cloned())
            .filter(|name| name != BACKEND_CLASSICAL);

        for route in &mut solution.routes {
            if !policy.accepts(route.stops.len()) {
                continue;
            }
            let classical_distance = network.route_distance_km(&route.depot_id, &route.stops);
            let mut report = SequencingReport {
                vehicle_id: route.vehicle_id.clone(),
                stops: route.stops.len(),
                variables: route.stops.len() * route.stops.len(),
                backend: "classical".into(),
                classical_distance_km: classical_distance,
                qubo_distance_km: None,
                chosen: SequencingChoice::Classical,
                reason: String::new(),
                solver_runtime_ms: None,
            };

            let Some(backend_name) = backend_name.clone() else {
                report.reason = "no sampling backend is available for sequencing".into();
                reports.push(report);
                continue;
            };
            let Ok(backend) = context.registry.require(&backend_name) else {
                report.reason = format!("backend '{backend_name}' is not registered");
                reports.push(report);
                continue;
            };
            let Ok(model) = tsp_problem(network, &route.depot_id, &route.stops) else {
                report.reason = "the route is too short to sequence".into();
                reports.push(report);
                continue;
            };

            report.backend = backend_name.clone();
            let decision_problem = seeded_problem(
                DecisionProblem::new(format!("sequence-{}", route.vehicle_id))
                    .with_optimization(model.clone()),
                request.options.sampler_seed,
            );
            let solver_context = SolverContext::from_task(
                context
                    .task
                    .as_ref()
                    .unwrap_or(&AgentTask::new("sequence a vehicle route")),
            );

            match backend.solve(decision_problem, solver_context).await {
                Ok(output) => {
                    report.solver_runtime_ms = output
                        .telemetry
                        .provider_metadata
                        .as_ref()
                        .and_then(|value| value.get("solver_runtime_ms"))
                        .and_then(|value| value.as_f64());

                    match output
                        .solution
                        .as_ref()
                        .and_then(|solution| decode_sequence(solution, &model))
                    {
                        Some(sequence) => {
                            let distance = network.route_distance_km(&route.depot_id, &sequence);
                            report.qubo_distance_km = Some(distance);
                            if distance + 1e-9 < classical_distance {
                                report.chosen = SequencingChoice::Qubo;
                                report.reason = format!(
                                    "sampled tour is shorter: {distance:.3} km against {classical_distance:.3} km"
                                );
                                route.stops = sequence;
                            } else {
                                report.reason = format!(
                                    "classical tour is no worse: {classical_distance:.3} km against {distance:.3} km"
                                );
                            }
                        }
                        None => {
                            report.reason =
                                "the sample was not a valid tour, so the classical route stands"
                                    .into();
                        }
                    }
                }
                Err(error) => {
                    report.reason = format!("backend '{backend_name}' failed: {error}");
                }
            }

            reports.push(report);
        }

        reports
    }

    /// Records how each subproblem went, so later runs can route empirically.
    pub fn ledger_records(reports: &[SubproblemReport]) -> Vec<LedgerRecord> {
        reports
            .iter()
            .filter(|report| report.objective.is_some())
            .map(|report| LedgerRecord {
                class: report.class.clone(),
                size_bucket: crate::quantumclaw_brains_router::routing_policy::size_bucket(
                    report.variables,
                ),
                backend: report.backend.clone(),
                objective: report.objective.unwrap_or_default(),
                feasible: report.feasible,
                runtime_ms: report.runtime_ms,
            })
            .collect()
    }
}

#[async_trait]
impl QuantumBrain for QRouterBrain {
    type Input = QRouterRequest;
    type Output = QRouterResult;

    fn id(&self) -> &str {
        "qrouter"
    }

    fn capabilities(&self) -> BrainCapabilities {
        BrainCapabilities::new("qrouter", "logistics")
            .with_problem_class("vehicle-assignment")
            .with_problem_class("capacitated-vehicle-routing")
            .with_problem_class("vehicle-routing-with-time-windows")
            .with_decomposition(true)
    }

    fn can_handle(&self, task: &AgentTask) -> BrainMatch {
        let keywords: Vec<String> = DOMAIN_KEYWORDS
            .iter()
            .map(|keyword| (*keyword).to_string())
            .collect();
        BrainMatch::from_keywords(&task.description, &keywords)
    }

    async fn validate(&self, input: &Self::Input) -> Result<ValidationReport> {
        let problem = &input.problem;
        let mut report = ValidationReport::default();

        if problem.depots.is_empty() {
            report.error("depots", "a routing problem needs at least one depot");
        }
        if problem.vehicles.is_empty() {
            report.error("vehicles", "a routing problem needs at least one vehicle");
        }
        if problem.deliveries.is_empty() {
            report.error("deliveries", "there is nothing to deliver");
        }

        for vehicle in &problem.vehicles {
            if problem.depot(&vehicle.depot_id).is_none() {
                report.error(
                    vehicle.id.clone(),
                    format!(
                        "vehicle is stationed at unknown depot '{}'",
                        vehicle.depot_id
                    ),
                );
            }
            if vehicle.capacity == 0 {
                report.error(vehicle.id.clone(), "vehicle capacity must be positive");
            }
        }

        let largest_capacity = problem
            .vehicles
            .iter()
            .map(|vehicle| vehicle.capacity)
            .max()
            .unwrap_or(0);

        for delivery in &problem.deliveries {
            if delivery.demand > largest_capacity {
                report.error(
                    delivery.id.clone(),
                    format!(
                        "demand of {} exceeds the largest vehicle capacity of {largest_capacity}",
                        delivery.demand
                    ),
                );
            }
            if let Some(depot_id) = &delivery.depot_id {
                if problem.depot(depot_id).is_none() {
                    report.error(
                        delivery.id.clone(),
                        format!("delivery is restricted to unknown depot '{depot_id}'"),
                    );
                }
            }
            if let Some(window) = delivery.window {
                if window.end_min < window.start_min {
                    report.error(
                        delivery.id.clone(),
                        "delivery window closes before it opens",
                    );
                }
            }
        }

        if problem.total_demand() > problem.total_capacity() {
            report.error(
                "fleet",
                format!(
                    "total demand of {} exceeds total fleet capacity of {}",
                    problem.total_demand(),
                    problem.total_capacity()
                ),
            );
        }

        // Surfaces matrix problems such as a mis-sized explicit matrix.
        if let Err(error) = Network::build(problem) {
            report.error("matrix", error.to_string());
        }

        Ok(report.finish())
    }

    async fn plan(&self, input: &Self::Input) -> Result<BrainPlan> {
        let network = Network::build(&input.problem)?;
        let (subproblems, summary) = self.decompose_problem(input, &network)?;
        let largest = subproblems
            .iter()
            .map(|piece| piece.variable_estimate())
            .max()
            .unwrap_or(0);

        Ok(BrainPlan::default()
            .stage(
                "validate",
                "check depots, fleet, demands, and windows",
                None,
            )
            .stage(
                "decompose",
                format!(
                    "split into {} subproblem(s) using {}",
                    subproblems.len(),
                    summary.strategy
                ),
                None,
            )
            .stage(
                "assign",
                format!("solve vehicle assignment, at most {largest} binary variables per piece"),
                Some("sampling backend or classical greedy".into()),
            )
            .stage(
                "sequence",
                "order each vehicle's stops with nearest neighbour and 2-opt",
                Some("classical".into()),
            )
            .stage(
                "evaluate",
                "re-check constraints and compute logistics KPIs",
                None,
            ))
    }

    async fn formulate(&self, input: &Self::Input) -> Result<Vec<Formulation>> {
        let network = Network::build(&input.problem)?;
        let (subproblems, _) = self.decompose_problem(input, &network)?;

        subproblems
            .iter()
            .map(|subproblem| {
                let problem =
                    assignment_problem(&input.problem, &network, subproblem, &self.weights)?;
                let mut metadata = BTreeMap::new();
                metadata.insert("depot".into(), subproblem.depot_id.clone());
                metadata.insert(
                    "deliveries".into(),
                    subproblem.delivery_ids.len().to_string(),
                );
                metadata.insert("vehicles".into(), subproblem.vehicle_ids.len().to_string());
                Ok(Formulation {
                    id: subproblem.id.clone(),
                    class: subproblem.class.as_str().to_string(),
                    problem,
                    metadata,
                })
            })
            .collect()
    }

    async fn decompose(&self, input: &Self::Input) -> Result<Decomposition> {
        let network = Network::build(&input.problem)?;
        Ok(self.decompose_problem(input, &network)?.1)
    }

    async fn solve(&self, input: Self::Input, context: BrainSolveContext) -> Result<Self::Output> {
        let started = Instant::now();
        self.validate(&input).await?.into_result()?;

        let network = Network::build(&input.problem)?;
        let (subproblems, summary) = self.decompose_problem(&input, &network)?;

        let mut assignment = Assignment::new();
        let mut reports = Vec::new();
        for subproblem in &subproblems {
            let (partial, report) = self
                .solve_subproblem(&input, &network, subproblem, &context)
                .await?;
            assignment.extend(partial);
            reports.push(report);
        }

        let all_vehicles: Vec<String> = input
            .problem
            .vehicles
            .iter()
            .map(|vehicle| vehicle.id.clone())
            .collect();
        let (assignment, unassigned) = repair(&input.problem, &network, &all_vehicles, assignment);
        let mut solution = build_solution(&input.problem, &network, &assignment, unassigned);

        // Optional: let a sampler try to beat the classical tour. It can only
        // replace a route by producing a strictly shorter valid one.
        let sequencing = self
            .sequence_with_sampler(&input, &network, &mut solution, &context)
            .await;

        let violations = solution_violations(&input.problem, &network, &solution);
        let runtime_ms = started.elapsed().as_millis() as u64;
        let solver_runtime_ms = reports
            .iter()
            .filter_map(|report| report.solver_runtime_ms)
            .reduce(|left, right| left + right);
        let kpis = kpis::evaluate(
            &input.problem,
            &network,
            &solution,
            runtime_ms,
            solver_runtime_ms,
        );

        Ok(QRouterResult {
            problem_id: input.problem.id.clone(),
            feasible: violations.is_empty(),
            problem: input.problem,
            solution,
            kpis,
            decomposition: summary,
            subproblems: reports,
            sequencing,
            violations,
            runtime_ms,
        })
    }

    async fn evaluate(&self, output: &Self::Output) -> Result<KpiReport> {
        Ok(output.kpis.to_report())
    }

    async fn explain(&self, output: &Self::Output) -> Result<Explanation> {
        let backends: Vec<String> = output
            .subproblems
            .iter()
            .map(|report| report.backend.clone())
            .collect();

        let mut explanation = Explanation::new(format!(
            "Served {} of {} deliveries with {} of {} vehicles over {:.1} km, costing {:.2} with {} late.",
            output.kpis.deliveries_served,
            output.kpis.deliveries_served + output.kpis.unassigned_deliveries,
            output.kpis.vehicles_used,
            output.kpis.vehicles_available,
            output.kpis.total_distance_km,
            output.kpis.objective_value,
            output.kpis.late_deliveries,
        ))
        .detail(format!(
            "Decomposition: {} ({})",
            output.decomposition.strategy, output.decomposition.rationale
        ))
        .detail(format!("Assignment solved by: {}", backends.join(", ")));

        for report in &output.subproblems {
            explanation = explanation.detail(format!(
                "{}: {} variables, backend '{}' — {}",
                report.id, report.variables, report.backend, report.routing_reason
            ));
            if let Some(reason) = &report.fallback_reason {
                explanation =
                    explanation.detail(format!("{}: fell back because {reason}", report.id));
            }
        }

        for violation in &output.violations {
            explanation = explanation.detail(format!(
                "violation [{:?}] {}: {}",
                violation.kind, violation.subject, violation.description
            ));
        }

        Ok(explanation)
    }
}

/// Attaches a sampler seed so a stochastic backend can be reproduced.
fn seeded_problem(mut problem: DecisionProblem, seed: Option<u64>) -> DecisionProblem {
    if let Some(seed) = seed {
        problem.metadata.data.insert(
            crate::quantumclaw_core::hints::SAMPLER_SEED.into(),
            seed.to_string(),
        );
    }
    problem
}

/// Subproblem classes this brain produces.
pub fn supported_classes() -> Vec<SubproblemClass> {
    vec![SubproblemClass::VehicleAssignment]
}

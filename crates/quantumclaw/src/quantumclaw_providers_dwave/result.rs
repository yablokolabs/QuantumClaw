//! Maps bridge output onto QuantumClaw's normalized solver result.

use crate::quantumclaw_core::{
    BackendTelemetry, SolverKind, SolverOutput, SolverPlanStep, SolverScore,
};
use crate::quantumclaw_ir::optimization::{ObjectiveSense, OptimizationSolution};
use crate::quantumclaw_optimization::CompiledModel;
use crate::quantumclaw_providers_dwave::bridge::BridgeExecution;
use crate::quantumclaw_providers_dwave::models::BridgeResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const PROVIDER: &str = "dwave";

/// Structured run metadata for a single D-Wave execution.
///
/// Every field a given backend cannot measure stays `None` rather than being
/// filled with a plausible-looking number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DWaveRunMetadata {
    pub provider: String,
    /// `simulated_annealing`, `simulated_quantum_annealing`, `exact`, `hybrid`, or `qpu`.
    pub backend: String,
    pub sampler: String,
    pub problem_type: String,
    pub variables: usize,
    pub interactions: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_reads: Option<u32>,
    pub objective: f64,
    pub energy: f64,
    pub feasible: bool,
    pub violations: usize,
    /// Wall time measured by QuantumClaw, including process startup.
    pub runtime_ms: u64,
    /// Time spent inside the sampler, measured by the bridge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solver_runtime_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qpu_access_time_us: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hybrid_run_time_us: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_time_us: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_break_fraction: Option<f64>,
    /// Raw `sampleset.info` from Ocean, passed through untouched.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

impl DWaveRunMetadata {
    /// Reads provider metadata back out of solver telemetry.
    pub fn from_telemetry(telemetry: &BackendTelemetry) -> Option<Self> {
        telemetry
            .provider_metadata
            .as_ref()
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }
}

/// Builds the normalized solver output for a completed bridge run.
pub fn to_solver_output(
    backend_name: &str,
    kind: SolverKind,
    model: &CompiledModel,
    execution: &BridgeExecution,
) -> SolverOutput {
    let BridgeExecution {
        result,
        total_runtime_ms,
    } = execution;

    let solution = model.decode(&result.best.sample);
    let metadata = run_metadata(result, &solution, *total_runtime_ms);
    let steps = plan_steps(model, &solution);

    let mut telemetry = BackendTelemetry::new(backend_name.to_string(), kind)
        .with_provider(PROVIDER)
        .with_provider_metadata(serde_json::to_value(&metadata).unwrap_or(Value::Null));
    telemetry.latency_ms = *total_runtime_ms;
    telemetry.confidence = confidence(backend_name, &solution);
    telemetry.notes.push(format!(
        "{} returned an energy of {:.6} over {} variables and {} interactions",
        result.sampler, result.best.energy, result.num_variables, result.num_interactions
    ));
    if !solution.feasible {
        telemetry.notes.push(format!(
            "the returned sample violates {} hard constraint(s)",
            solution.hard_violations().count()
        ));
    }

    let score = SolverScore {
        utility: comparable_utility(&solution),
        confidence: telemetry.confidence,
        cost_estimate: 0.0,
        risk: if solution.feasible { 0.0 } else { 1.0 },
    };

    SolverOutput {
        backend: backend_name.to_string(),
        backend_kind: kind,
        steps,
        score,
        rationale: rationale(backend_name, result, &solution),
        telemetry,
        solution: Some(solution),
    }
}

/// Objective oriented so that higher is always better, for planner comparison.
/// The unmodified objective stays available on the solution itself.
fn comparable_utility(solution: &OptimizationSolution) -> f64 {
    match solution.sense {
        ObjectiveSense::Maximize => solution.objective_value,
        ObjectiveSense::Minimize => -solution.objective_value,
    }
}

/// Exhaustive search returns a proven optimum; heuristic samplers do not.
fn confidence(backend_name: &str, solution: &OptimizationSolution) -> f64 {
    match (backend_name, solution.feasible) {
        (_, false) => 0.2,
        ("dwave-exact", true) => 1.0,
        (_, true) => 0.85,
    }
}

fn rationale(backend_name: &str, result: &BridgeResult, solution: &OptimizationSolution) -> String {
    let feasibility = if solution.feasible {
        "feasible".to_string()
    } else {
        format!(
            "infeasible ({} hard violation(s))",
            solution.hard_violations().count()
        )
    };
    format!(
        "{backend_name} sampled the compiled QUBO with {} and returned a {feasibility} solution with objective {:.6}",
        result.sampler, solution.objective_value
    )
}

/// Renders selected variables as plan steps so plan-shaped consumers such as
/// the policy engine keep working with optimization backends.
fn plan_steps(model: &CompiledModel, solution: &OptimizationSolution) -> Vec<SolverPlanStep> {
    solution
        .selected
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let variable = model.problem().variable(name);
            let metadata: &BTreeMap<String, String> = match variable {
                Some(variable) => &variable.metadata,
                None => const { &BTreeMap::new() },
            };
            SolverPlanStep {
                id: format!("dwave-step-{index}"),
                action_id: metadata
                    .get("action")
                    .cloned()
                    .or_else(|| Some(name.clone())),
                title: metadata
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| name.clone()),
                tool_hint: metadata.get("tool_hint").cloned(),
                rationale: format!("selected by the compiled binary model as '{name}'"),
                expected_utility: 1.0,
                risk: if solution.feasible { 0.0 } else { 0.5 },
            }
        })
        .collect()
}

fn run_metadata(
    result: &BridgeResult,
    solution: &OptimizationSolution,
    total_runtime_ms: u64,
) -> DWaveRunMetadata {
    DWaveRunMetadata {
        provider: PROVIDER.into(),
        backend: result.backend.clone(),
        sampler: result.sampler.clone(),
        problem_type: result.problem_type.clone(),
        variables: result.num_variables,
        interactions: result.num_interactions,
        num_reads: result.num_reads,
        objective: solution.objective_value,
        energy: result.best.energy,
        feasible: solution.feasible,
        violations: solution.violations.len(),
        runtime_ms: total_runtime_ms,
        solver_runtime_ms: result.solver_runtime_ms,
        qpu_access_time_us: result.qpu_access_time_us,
        hybrid_run_time_us: result.run_time_us,
        charge_time_us: result.charge_time_us,
        chain_break_fraction: result.chain_break_fraction,
        metadata: result.info.clone(),
    }
}

use async_trait::async_trait;
use quantumclaw_core::{
    BackendTelemetry, Result, SolverBackend, SolverContext, SolverKind, SolverOutput,
    SolverPlanStep, SolverScore,
};
use quantumclaw_ir::{CandidateAction, DecisionProblem};
use std::time::Instant;

#[derive(Debug, Default, Clone)]
pub struct GreedySolver;

#[async_trait]
impl SolverBackend for GreedySolver {
    fn name(&self) -> &'static str {
        "greedy-classical"
    }
    fn kind(&self) -> SolverKind {
        SolverKind::Classical
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> Result<SolverOutput> {
        solve_classical(
            problem,
            self.name(),
            "Greedy selection ordered by dependency shape, risk, and expected utility",
        )
        .await
    }
}

#[derive(Debug, Clone)]
pub struct BeamSearchSolver {
    pub beam_width: usize,
}

impl Default for BeamSearchSolver {
    fn default() -> Self {
        Self { beam_width: 4 }
    }
}

#[async_trait]
impl SolverBackend for BeamSearchSolver {
    fn name(&self) -> &'static str {
        "beam-search-classical"
    }
    fn kind(&self) -> SolverKind {
        SolverKind::Classical
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> Result<SolverOutput> {
        solve_classical(
            problem,
            self.name(),
            "Beam search stub using the shared classical scoring surface",
        )
        .await
    }
}

#[derive(Debug, Default, Clone)]
pub struct HeuristicSearchSolver;

#[async_trait]
impl SolverBackend for HeuristicSearchSolver {
    fn name(&self) -> &'static str {
        "heuristic-search-classical"
    }
    fn kind(&self) -> SolverKind {
        SolverKind::Classical
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> Result<SolverOutput> {
        solve_classical(
            problem,
            self.name(),
            "Heuristic search stub with pluggable domain-agnostic scoring",
        )
        .await
    }
}

#[derive(Debug, Default, Clone)]
pub struct BranchAndBoundSolver;

#[async_trait]
impl SolverBackend for BranchAndBoundSolver {
    fn name(&self) -> &'static str {
        "branch-and-bound-classical"
    }
    fn kind(&self) -> SolverKind {
        SolverKind::Classical
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> Result<SolverOutput> {
        solve_classical(
            problem,
            self.name(),
            "Branch-and-bound stub preserving the SolverBackend contract",
        )
        .await
    }
}

#[derive(Debug, Default, Clone)]
pub struct SimulatedAnnealingSolver;

#[async_trait]
impl SolverBackend for SimulatedAnnealingSolver {
    fn name(&self) -> &'static str {
        "simulated-annealing-classical"
    }
    fn kind(&self) -> SolverKind {
        SolverKind::Classical
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> Result<SolverOutput> {
        solve_classical(
            problem,
            self.name(),
            "Simulated annealing stub with deterministic placeholder output",
        )
        .await
    }
}

#[derive(Debug, Default, Clone)]
pub struct EvolutionarySolver;

#[async_trait]
impl SolverBackend for EvolutionarySolver {
    fn name(&self) -> &'static str {
        "evolutionary-classical"
    }
    fn kind(&self) -> SolverKind {
        SolverKind::Classical
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> Result<SolverOutput> {
        solve_classical(
            problem,
            self.name(),
            "Evolutionary solver stub with reusable trait boundary",
        )
        .await
    }
}

async fn solve_classical(
    problem: DecisionProblem,
    backend: &'static str,
    rationale: &'static str,
) -> Result<SolverOutput> {
    let started = Instant::now();
    let mut actions = problem.candidate_actions.clone();
    actions.sort_by(|a, b| classical_score(b).total_cmp(&classical_score(a)));

    let mut steps: Vec<SolverPlanStep> = if actions.is_empty() {
        problem
            .subtasks
            .iter()
            .enumerate()
            .map(|(idx, subtask)| {
                let mut step =
                    SolverPlanStep::new(format!("step-{idx}"), subtask.description.clone());
                step.rationale =
                    "Generated from subtask because no candidate action was supplied".into();
                step
            })
            .collect()
    } else {
        actions.iter().enumerate().map(action_to_step).collect()
    };

    order_by_dependencies(&mut steps, &problem);
    let utility = steps.iter().map(|s| s.expected_utility).sum::<f64>() / steps.len().max(1) as f64;
    let risk = steps.iter().map(|s| s.risk).sum::<f64>() / steps.len().max(1) as f64;
    let confidence = (0.72 + utility * 0.2 - risk * 0.1).clamp(0.0, 0.99);

    let mut telemetry = BackendTelemetry::new(backend, SolverKind::Classical);
    telemetry.latency_ms = started.elapsed().as_millis() as u64;
    telemetry.confidence = confidence;
    telemetry.cost_estimate = problem.cost_model.estimated_money_micros as f64 / 1_000_000.0;
    telemetry.notes.push(rationale.into());

    Ok(SolverOutput {
        backend: backend.into(),
        backend_kind: SolverKind::Classical,
        steps,
        score: SolverScore {
            utility,
            confidence,
            cost_estimate: telemetry.cost_estimate,
            risk,
        },
        rationale: rationale.into(),
        telemetry,
        solution: None,
    })
}

fn action_to_step((idx, action): (usize, &CandidateAction)) -> SolverPlanStep {
    SolverPlanStep {
        id: format!("step-{idx}"),
        action_id: Some(action.id.clone()),
        title: action.name.clone(),
        tool_hint: action.tool_hint.clone(),
        rationale: action.description.clone(),
        expected_utility: action.utility.0,
        risk: action.risk.score,
    }
}

fn classical_score(action: &CandidateAction) -> f64 {
    action.utility.0 - action.risk.score - action.cost.estimated_latency_ms as f64 / 100_000.0
}

fn order_by_dependencies(steps: &mut [SolverPlanStep], problem: &DecisionProblem) {
    if problem.dependencies.is_empty() {
        return;
    }

    steps.sort_by_key(|step| {
        let action_id = step.action_id.as_deref().unwrap_or_default();
        problem
            .dependencies
            .iter()
            .position(|dep| dep.before == action_id)
            .unwrap_or(usize::MAX)
    });
}

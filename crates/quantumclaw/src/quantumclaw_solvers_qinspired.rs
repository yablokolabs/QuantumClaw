use crate::quantumclaw_core::{
    BackendTelemetry, Result, SolverBackend, SolverContext, SolverKind, SolverOutput,
    SolverPlanStep, SolverScore,
};
use crate::quantumclaw_ir::{CandidateAction, DecisionProblem};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuboLikeProblem {
    pub variables: Vec<String>,
    pub linear_weights: Vec<f64>,
    pub pairwise_penalties: Vec<(usize, usize, f64)>,
}

impl QuboLikeProblem {
    pub fn from_decision_problem(problem: &DecisionProblem) -> Self {
        Self {
            variables: problem
                .candidate_actions
                .iter()
                .map(|action| action.id.clone())
                .collect(),
            linear_weights: problem
                .candidate_actions
                .iter()
                .map(hybrid_heuristic_score)
                .collect(),
            pairwise_penalties: problem
                .dependencies
                .iter()
                .enumerate()
                .map(|(idx, _)| (idx, idx + 1, -0.1))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IsingLikeMapping {
    pub spins: Vec<String>,
    pub fields: Vec<f64>,
    pub couplings: Vec<(usize, usize, f64)>,
}

impl From<&QuboLikeProblem> for IsingLikeMapping {
    fn from(value: &QuboLikeProblem) -> Self {
        Self {
            spins: value.variables.clone(),
            fields: value.linear_weights.clone(),
            couplings: value.pairwise_penalties.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct QuantumInspiredSolver {
    pub exploration_weight: f64,
}

impl Default for QuantumInspiredSolver {
    fn default() -> Self {
        Self {
            exploration_weight: 0.15,
        }
    }
}

#[async_trait]
impl SolverBackend for QuantumInspiredSolver {
    fn name(&self) -> &'static str {
        "quantum-inspired-hybrid"
    }
    fn kind(&self) -> SolverKind {
        SolverKind::QuantumInspired
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> Result<SolverOutput> {
        let started = Instant::now();
        let qubo = QuboLikeProblem::from_decision_problem(&problem);
        let mapping = IsingLikeMapping::from(&qubo);
        let mut actions = problem.candidate_actions.clone();
        actions.sort_by(|a, b| hybrid_heuristic_score(b).total_cmp(&hybrid_heuristic_score(a)));

        let steps: Vec<SolverPlanStep> = if actions.is_empty() {
            problem
                .subtasks
                .iter()
                .enumerate()
                .map(|(idx, subtask)| {
                    let mut step =
                        SolverPlanStep::new(format!("qstep-{idx}"), subtask.description.clone());
                    step.rationale =
                        "Generated from subtask using q-inspired fallback scoring".into();
                    step
                })
                .collect()
        } else {
            actions
                .iter()
                .enumerate()
                .map(|(idx, action)| SolverPlanStep {
                    id: format!("qstep-{idx}"),
                    action_id: Some(action.id.clone()),
                    title: action.name.clone(),
                    tool_hint: action.tool_hint.clone(),
                    rationale: format!(
                        "Hybrid heuristic placeholder score {:.3}: {}",
                        hybrid_heuristic_score(action),
                        action.description
                    ),
                    expected_utility: action.utility.0 + self.exploration_weight.min(0.2) * 0.05,
                    risk: action.risk.score,
                })
                .collect()
        };

        let utility =
            steps.iter().map(|s| s.expected_utility).sum::<f64>() / steps.len().max(1) as f64;
        let risk = steps.iter().map(|s| s.risk).sum::<f64>() / steps.len().max(1) as f64;
        let confidence = (0.7_f64 + utility * 0.22 - risk * 0.08).clamp(0.0, 0.99);

        let mut telemetry = BackendTelemetry::new(self.name(), SolverKind::QuantumInspired);
        telemetry.latency_ms = started.elapsed().as_millis() as u64;
        telemetry.confidence = confidence;
        telemetry.cost_estimate = mapping.couplings.len() as f64 * 0.00001;
        telemetry.notes.push("QUBO-like and Ising-like mapping placeholders were constructed without a hardware SDK dependency".into());

        Ok(SolverOutput {
            backend: self.name().into(),
            backend_kind: SolverKind::QuantumInspired,
            steps,
            score: SolverScore { utility, confidence, cost_estimate: telemetry.cost_estimate, risk },
            rationale: "Quantum-inspired solver stub using hybrid heuristic scoring over backend-neutral IR".into(),
            telemetry,
        })
    }
}

fn hybrid_heuristic_score(action: &CandidateAction) -> f64 {
    action.utility.0 * 0.76
        - action.risk.score * 0.18
        - action.cost.estimated_latency_ms as f64 / 150_000.0
}

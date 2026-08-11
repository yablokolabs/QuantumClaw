//! Backend selection by name, and what ShadowCompare reports about two solvers.

use async_trait::async_trait;
use quantumclaw_core::{
    AgentTask, BackendTelemetry, Result, SolverBackend, SolverContext, SolverKind, SolverOutput,
    SolverScore,
};
use quantumclaw_ir::optimization::{ConstraintViolation, ObjectiveSense, OptimizationSolution};
use quantumclaw_ir::DecisionProblem;
use quantumclaw_planner::{
    BackendSelectionPolicy, ComparisonVerdict, HybridPlanner, PlannerMode, PlannerRequest,
};
use std::collections::BTreeMap;
use std::sync::Arc;

/// A backend that returns a fixed optimization result, so the planner's own
/// comparison behavior is what the assertions measure.
struct FixedBackend {
    name: &'static str,
    kind: SolverKind,
    objective: f64,
    feasible: bool,
    solver_runtime_ms: f64,
}

impl FixedBackend {
    fn solution(&self) -> OptimizationSolution {
        OptimizationSolution {
            problem_id: "fixed".into(),
            assignments: BTreeMap::new(),
            selected: vec!["x".into()],
            objective_value: self.objective,
            energy: self.objective,
            sense: ObjectiveSense::Minimize,
            feasible: self.feasible,
            violations: if self.feasible {
                Vec::new()
            } else {
                vec![ConstraintViolation {
                    constraint_id: "capacity".into(),
                    description: "over capacity".into(),
                    magnitude: 3.0,
                    hard: true,
                }]
            },
            metadata: BTreeMap::new(),
        }
    }
}

#[async_trait]
impl SolverBackend for FixedBackend {
    fn name(&self) -> &'static str {
        self.name
    }

    fn kind(&self) -> SolverKind {
        self.kind
    }

    async fn solve(
        &self,
        _problem: DecisionProblem,
        _context: SolverContext,
    ) -> Result<SolverOutput> {
        let telemetry = BackendTelemetry::new(self.name, self.kind)
            .with_provider("test")
            .with_provider_metadata(serde_json::json!({
                "solver_runtime_ms": self.solver_runtime_ms
            }));
        Ok(SolverOutput {
            backend: self.name.into(),
            backend_kind: self.kind,
            steps: Vec::new(),
            score: SolverScore::default(),
            rationale: "fixed".into(),
            telemetry,
            solution: Some(self.solution()),
        })
    }
}

fn backend(
    name: &'static str,
    kind: SolverKind,
    objective: f64,
    feasible: bool,
) -> Arc<dyn SolverBackend> {
    Arc::new(FixedBackend {
        name,
        kind,
        objective,
        feasible,
        solver_runtime_ms: 12.5,
    })
}

fn problem() -> DecisionProblem {
    DecisionProblem::for_task("assign work")
}

#[tokio::test]
async fn an_explicitly_named_backend_wins_over_the_mode_preference() {
    let response = HybridPlanner::default()
        .plan(
            PlannerRequest::new(AgentTask::new("optimize"))
                .with_problem(problem())
                .with_mode(PlannerMode::ClassicalOnly)
                .with_selection_policy(BackendSelectionPolicy::prefer_backend("dwave-sa"))
                .with_backend(backend(
                    "greedy-classical",
                    SolverKind::Classical,
                    10.0,
                    true,
                ))
                .with_backend(backend("dwave-sa", SolverKind::Classical, 8.0, true)),
        )
        .await
        .expect("planning succeeds");

    assert_eq!(response.primary_plan().backend, "dwave-sa");
}

#[tokio::test]
async fn requesting_a_backend_that_is_not_registered_fails_loudly() {
    let error = HybridPlanner::default()
        .plan(
            PlannerRequest::new(AgentTask::new("optimize"))
                .with_problem(problem())
                .with_selection_policy(BackendSelectionPolicy::prefer_backend("dwave-qpu"))
                .with_backend(backend(
                    "greedy-classical",
                    SolverKind::Classical,
                    10.0,
                    true,
                )),
        )
        .await
        .map(|_| ())
        .expect_err("an unavailable backend must not be silently ignored");

    let message = error.to_string();
    assert!(message.contains("dwave-qpu"), "{message}");
    assert!(message.contains("greedy-classical"), "{message}");
}

#[tokio::test]
async fn shadow_compare_reports_which_solver_produced_the_better_objective() {
    let response = HybridPlanner::default()
        .plan(
            PlannerRequest::new(AgentTask::new("optimize"))
                .with_problem(problem())
                .with_mode(PlannerMode::ShadowCompare)
                .with_selection_policy(
                    BackendSelectionPolicy::prefer_backend("classical-production")
                        .with_shadow(true),
                )
                .with_backend(backend(
                    "classical-production",
                    SolverKind::Classical,
                    100.0,
                    true,
                ))
                .with_shadow_backend(backend("dwave-sa", SolverKind::Classical, 80.0, true)),
        )
        .await
        .expect("planning succeeds");

    let comparison = response
        .telemetry
        .shadow_comparison
        .as_ref()
        .expect("shadow comparison is recorded");
    let optimization = comparison
        .optimization
        .as_ref()
        .expect("both backends returned optimization results");

    assert_eq!(optimization.primary_objective, 100.0);
    assert_eq!(optimization.shadow_objective, 80.0);
    assert_eq!(optimization.verdict, ComparisonVerdict::Shadow);
    assert!(
        (optimization.objective_delta - 20.0).abs() < 1e-9,
        "minimization improvement of 20 units, got {}",
        optimization.objective_delta
    );
    assert!(
        (optimization.relative_gap.expect("gap is computable") - 0.2).abs() < 1e-9,
        "20% better than the primary"
    );
    assert_eq!(optimization.primary_solver_runtime_ms, Some(12.5));
}

#[tokio::test]
async fn an_infeasible_shadow_result_never_wins_on_objective_alone() {
    let response = HybridPlanner::default()
        .plan(
            PlannerRequest::new(AgentTask::new("optimize"))
                .with_problem(problem())
                .with_mode(PlannerMode::ShadowCompare)
                .with_selection_policy(
                    BackendSelectionPolicy::prefer_backend("classical-production")
                        .with_shadow(true),
                )
                .with_backend(backend(
                    "classical-production",
                    SolverKind::Classical,
                    100.0,
                    true,
                ))
                // A far better objective, but it breaks a hard constraint.
                .with_shadow_backend(backend("dwave-sa", SolverKind::Classical, 1.0, false)),
        )
        .await
        .expect("planning succeeds");

    let optimization = response
        .telemetry
        .shadow_comparison
        .as_ref()
        .and_then(|comparison| comparison.optimization.as_ref())
        .expect("comparison is recorded");

    assert_eq!(optimization.verdict, ComparisonVerdict::Primary);
    assert!(!optimization.shadow_feasible);
    assert_eq!(optimization.shadow_violations, 1);
}

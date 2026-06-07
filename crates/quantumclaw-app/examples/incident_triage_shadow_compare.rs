use quantumclaw_core::{AgentTask, Result, SolverKind, TaskType};
use quantumclaw_ir::{CandidateAction, DecisionProblem, Dependency, Goal};
use quantumclaw_planner::{BackendSelectionPolicy, HybridPlanner, PlannerMode, PlannerRequest};
use quantumclaw_solvers_classical::GreedySolver;
use quantumclaw_solvers_qinspired::QuantumInspiredSolver;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let task = AgentTask::new(
        "Triage a failing production deploy: gather symptoms, identify likely owner, choose rollback or fix-forward, and define validation gates.",
    )
    .with_task_type(TaskType::Workflow)
    .with_latency_budget_ms(5_000);

    let mut problem = DecisionProblem::new("incident-triage");
    problem.goals.push(Goal::new(
        "restore-service",
        "Restore service health while preserving evidence for root-cause analysis.",
    ));
    problem.candidate_actions = vec![
        CandidateAction::new(
            "collect-symptoms",
            "Collect failing checks and customer-impact signals",
            "Read deploy logs, health checks, metrics, and recent alerts.",
        )
        .with_tool_hint("search")
        .with_utility(0.97),
        CandidateAction::new(
            "compare-deploy-diff",
            "Compare current deploy against last known-good release",
            "Identify code, config, migration, and dependency changes in the release window.",
        )
        .with_tool_hint("filesystem")
        .with_utility(0.93),
        CandidateAction::new(
            "choose-mitigation",
            "Choose rollback or fix-forward path",
            "Rank mitigation options by recovery time, blast radius, and reversibility.",
        )
        .with_tool_hint("external-api")
        .with_utility(0.90),
        CandidateAction::new(
            "validate-recovery",
            "Validate recovery gates before closing incident",
            "Confirm metrics, smoke tests, and user-impact signals recovered.",
        )
        .with_tool_hint("shell")
        .with_utility(0.88),
    ];
    problem.dependencies = vec![
        Dependency::new(
            "collect-symptoms",
            "compare-deploy-diff",
            "Diff analysis depends on knowing the failure window.",
        ),
        Dependency::new(
            "compare-deploy-diff",
            "choose-mitigation",
            "Mitigation choice depends on likely culprit and rollback safety.",
        ),
        Dependency::new(
            "choose-mitigation",
            "validate-recovery",
            "Validation follows the selected mitigation.",
        ),
    ];

    let planner = HybridPlanner::default();
    let response = planner
        .plan(
            PlannerRequest::new(task)
                .with_problem(problem)
                .with_mode(PlannerMode::ShadowCompare)
                .with_selection_policy(
                    BackendSelectionPolicy::prefer(SolverKind::QuantumInspired).with_shadow(true),
                )
                .with_backend(Arc::new(GreedySolver))
                .with_backend(Arc::new(QuantumInspiredSolver::default())),
        )
        .await?;

    let plan = response.primary_plan();
    println!("primary backend: {}", plan.backend);
    println!("utility: {:.2}", plan.score.utility);
    println!("confidence: {:.2}", plan.score.confidence);

    if let Some(comparison) = response.telemetry.shadow_comparison.as_ref() {
        println!("shadow backend: {}", comparison.shadow_backend);
        println!(
            "primary utility {:.2} vs shadow utility {:.2}",
            comparison.primary_score.utility, comparison.shadow_score.utility
        );
    }

    println!("triage plan:");
    for (index, step) in plan.steps.iter().enumerate() {
        println!("  {}. {} ({})", index + 1, step.title, step.tool_name);
    }
    Ok(())
}

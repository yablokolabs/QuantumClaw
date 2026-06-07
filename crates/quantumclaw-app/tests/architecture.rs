use std::sync::Arc;

use quantumclaw_app::run_refactor_demo;
use quantumclaw_core::{AgentTask, SolverBackend, SolverKind};
use quantumclaw_ir::DecisionProblem;
use quantumclaw_memory::{InMemoryProceduralMemory, ProceduralMemory, StoredProcedure};
use quantumclaw_planner::{BackendSelectionPolicy, HybridPlanner, PlannerMode, PlannerRequest};
use quantumclaw_planner::{Plan, PlanScore, PlanStep, PlannerRationale};
use quantumclaw_policy::{DeterministicPolicyEngine, PolicyDecision, RiskLevel};
use quantumclaw_solvers_classical::GreedySolver;
use quantumclaw_solvers_qinspired::QuantumInspiredSolver;

#[tokio::test]
async fn classical_and_quantum_inspired_backends_share_solver_trait() {
    let backends: Vec<Arc<dyn SolverBackend>> = vec![
        Arc::new(GreedySolver),
        Arc::new(QuantumInspiredSolver::default()),
    ];

    assert_eq!(backends[0].kind(), SolverKind::Classical);
    assert_eq!(backends[1].kind(), SolverKind::QuantumInspired);
}

#[tokio::test]
async fn planner_switches_backends_without_runtime_code_changes() {
    let planner = HybridPlanner::default();
    let task = AgentTask::new("plan a Rust refactor");

    let classical = planner
        .plan(
            PlannerRequest::new(task.clone())
                .with_mode(PlannerMode::ClassicalOnly)
                .with_backend(Arc::new(GreedySolver))
                .with_backend(Arc::new(QuantumInspiredSolver::default())),
        )
        .await
        .unwrap();

    let qinspired = planner
        .plan(
            PlannerRequest::new(task)
                .with_mode(PlannerMode::QuantumInspiredPreferred)
                .with_backend(Arc::new(GreedySolver))
                .with_backend(Arc::new(QuantumInspiredSolver::default())),
        )
        .await
        .unwrap();

    assert_eq!(classical.primary_plan().backend_kind, SolverKind::Classical);
    assert_eq!(
        qinspired.primary_plan().backend_kind,
        SolverKind::QuantumInspired
    );
}

#[test]
fn ir_is_backend_neutral() {
    let problem = DecisionProblem::for_task("safe Rust refactor");
    let serialized = serde_json::to_string(&problem).unwrap().to_lowercase();

    assert!(!serialized.contains("classical"));
    assert!(!serialized.contains("quantum"));
    assert!(!serialized.contains("qpu"));
}

#[tokio::test]
async fn policy_rejects_risky_plan() {
    let policy = DeterministicPolicyEngine::default();
    let plan = Plan {
        id: "risky".into(),
        backend: "test".into(),
        backend_kind: SolverKind::Classical,
        steps: vec![PlanStep::new("delete workspace", "shell").with_risk(RiskLevel::Critical)],
        score: PlanScore::default(),
        rationale: PlannerRationale::new("test plan"),
        metadata: Default::default(),
    };

    let decision: PolicyDecision = policy.evaluate_plan(&plan).await.unwrap();
    assert!(!decision.allowed);
    assert!(decision.reasons.iter().any(|r| r.contains("critical")));
}

#[tokio::test]
async fn procedural_memory_stores_and_retrieves_learned_skill() {
    let memory = InMemoryProceduralMemory::default();
    memory
        .store_procedure(StoredProcedure::new(
            "safe-rust-refactor",
            "Use tests, small edits, and validation for Rust refactors",
            ["rust", "refactor", "tests"],
        ))
        .await
        .unwrap();

    let matches = memory
        .retrieve_similar("rust module refactor with tests", 3)
        .await
        .unwrap();
    assert_eq!(matches[0].id, "safe-rust-refactor");
}

#[tokio::test]
async fn shadow_compare_records_primary_and_shadow_telemetry() {
    let planner = HybridPlanner::default();
    let response = planner
        .plan(
            PlannerRequest::new(AgentTask::new("compare planning backends"))
                .with_mode(PlannerMode::ShadowCompare)
                .with_selection_policy(
                    BackendSelectionPolicy::prefer(SolverKind::Classical).with_shadow(true),
                )
                .with_backend(Arc::new(GreedySolver))
                .with_backend(Arc::new(QuantumInspiredSolver::default())),
        )
        .await
        .unwrap();

    let comparison = response
        .telemetry
        .shadow_comparison
        .as_ref()
        .expect("shadow comparison telemetry");
    assert_eq!(comparison.primary_backend_kind, SolverKind::Classical);
    assert_eq!(comparison.shadow_backend_kind, SolverKind::QuantumInspired);
    assert_eq!(response.primary_plan().backend_kind, SolverKind::Classical);
}

#[tokio::test]
async fn generic_task_flow_has_no_home_automation_dependency() {
    let report = run_refactor_demo().await.unwrap();
    let text = format!(
        "{} {:?} {:?}",
        report.session.user_task, report.plan.steps, report.learned_skill.title
    )
    .to_lowercase();

    for forbidden in [
        "iot",
        "home automation",
        "smart home",
        "thermostat",
        "light bulb",
    ] {
        assert!(
            !text.contains(forbidden),
            "flow leaked domain term: {forbidden}"
        );
    }
    assert!(text.contains("refactor"));
}

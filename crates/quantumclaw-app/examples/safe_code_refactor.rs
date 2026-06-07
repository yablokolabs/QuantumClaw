use quantumclaw_core::{Result, SolverBackend};
use quantumclaw_memory::{InMemoryProceduralMemory, ProceduralMemory, StoredProcedure};
use quantumclaw_observability::InMemoryObserver;
use quantumclaw_policy::DeterministicPolicyEngine;
use quantumclaw_runtime::QuantumClawRuntime;
use quantumclaw_solvers_classical::GreedySolver;
use quantumclaw_solvers_qinspired::QuantumInspiredSolver;
use quantumclaw_tools::InMemoryToolRegistry;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let memory = InMemoryProceduralMemory::default();
    memory
        .store_procedure(StoredProcedure::new(
            "safe-rust-refactor",
            "Inspect interfaces first, preserve tests, make small reversible edits, run fmt, clippy, and tests before declaring success.",
            ["rust", "refactor", "tests", "clippy", "validation"],
        ))
        .await?;

    let runtime = QuantumClawRuntime::new(
        vec![
            Arc::new(GreedySolver) as Arc<dyn SolverBackend>,
            Arc::new(QuantumInspiredSolver::default()),
        ],
        memory,
        InMemoryToolRegistry::with_default_tools(),
        DeterministicPolicyEngine::default(),
        InMemoryObserver::default(),
    );

    let report = runtime
        .handle_user_task(
            "Refactor the memory ranking module without changing public behavior, then validate it.",
        )
        .await?;

    println!("task: {}", report.session.user_task);
    println!("selected backend: {}", report.plan.backend);
    println!("policy allowed: {}", report.policy_decision.allowed);
    println!(
        "retrieved procedures: {}",
        report.retrieved_procedures.len()
    );
    println!("plan steps:");
    for (index, step) in report.plan.steps.iter().enumerate() {
        println!(
            "  {}. [{}] {} via {}",
            index + 1,
            step.risk_level,
            step.title,
            step.tool_name
        );
    }
    println!("learned skill: {}", report.learned_skill.id);
    Ok(())
}

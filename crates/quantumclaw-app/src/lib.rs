pub mod wiring;

pub use wiring::{brain_registry, solver_registry, tool_registry};

use quantumclaw_core::{Result, SolverBackend};
use quantumclaw_memory::{InMemoryProceduralMemory, ProceduralMemory, StoredProcedure};
use quantumclaw_observability::InMemoryObserver;
use quantumclaw_policy::DeterministicPolicyEngine;
use quantumclaw_runtime::{QuantumClawRuntime, RuntimeExecutionReport};
use quantumclaw_solvers_classical::GreedySolver;
use quantumclaw_solvers_qinspired::QuantumInspiredSolver;
use quantumclaw_tools::InMemoryToolRegistry;
use std::sync::Arc;

pub async fn run_refactor_demo() -> Result<RuntimeExecutionReport> {
    let memory = InMemoryProceduralMemory::default();
    memory.store_procedure(StoredProcedure::new(
        "safe-rust-refactor",
        "When refactoring Rust code, inspect interfaces, add or preserve tests, make small reversible edits, and run formatter plus tests.",
        ["rust", "refactor", "tests", "validation"],
    )).await?;

    let backends: Vec<Arc<dyn SolverBackend>> = vec![
        Arc::new(GreedySolver),
        Arc::new(QuantumInspiredSolver::default()),
    ];
    let runtime = QuantumClawRuntime::new(
        backends,
        memory,
        InMemoryToolRegistry::with_default_tools(),
        DeterministicPolicyEngine::default(),
        InMemoryObserver::default(),
    );

    runtime
        .handle_user_task("Plan a safe coding refactor for a Rust module.")
        .await
}

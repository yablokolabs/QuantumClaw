use async_trait::async_trait;
use quantumclaw_core::{
    QuantumClawError, Result, SolverBackend, SolverContext, SolverKind, SolverOutput,
};
use quantumclaw_ir::DecisionProblem;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FutureQpuJobHandle {
    pub backend_name: String,
    pub job_id: String,
}

#[async_trait]
pub trait FutureQpuBackend: Send + Sync {
    async fn submit_problem(&self, problem: DecisionProblem) -> Result<FutureQpuJobHandle>;
    async fn poll_solution(&self, handle: &FutureQpuJobHandle) -> Result<Option<SolverOutput>>;
}

#[derive(Debug, Clone)]
pub struct FutureQpuSolverAdapter<B> {
    pub backend: B,
}

impl<B> FutureQpuSolverAdapter<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl<B> SolverBackend for FutureQpuSolverAdapter<B>
where
    B: FutureQpuBackend,
{
    fn name(&self) -> &'static str {
        "future-qpu-adapter"
    }
    fn kind(&self) -> SolverKind {
        SolverKind::FutureQpu
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> Result<SolverOutput> {
        let handle = self.backend.submit_problem(problem).await?;
        self.backend
            .poll_solution(&handle)
            .await?
            .ok_or_else(|| QuantumClawError::new("future QPU backend returned no solution yet"))
    }
}

#[derive(Debug, Default, Clone)]
pub struct PlaceholderFutureQpuBackend;

#[async_trait]
impl FutureQpuBackend for PlaceholderFutureQpuBackend {
    async fn submit_problem(&self, _problem: DecisionProblem) -> Result<FutureQpuJobHandle> {
        Err(QuantumClawError::new(
            "future QPU SDK integration is intentionally feature-gated and not linked",
        ))
    }

    async fn poll_solution(&self, _handle: &FutureQpuJobHandle) -> Result<Option<SolverOutput>> {
        Ok(None)
    }
}

#[cfg(feature = "future-sdk")]
pub mod sdk_integration {
    pub struct ExternalSdkClientPlaceholder;
}

//! D-Wave solver backends.
//!
//! All five implement the same [`SolverBackend`] contract as QuantumClaw's
//! classical and quantum-inspired solvers, so selecting one is a configuration
//! choice rather than a code change.

use crate::bridge::DWaveBridge;
use crate::config::{
    ExactParams, HybridParams, LeapConfig, QpuParams, SimulatedAnnealingParams,
    SimulatedQuantumAnnealingParams,
};
use crate::error::{DWaveError, Result};
use crate::models::{BridgeBackend, BridgeBqm, BridgeRequest};
use crate::result::to_solver_output;
use async_trait::async_trait;
use quantumclaw_core::{
    hints, Result as CoreResult, SolverBackend, SolverCapabilities, SolverContext, SolverKind,
    SolverOutput,
};
use quantumclaw_ir::DecisionProblem;
use quantumclaw_optimization::{optimization_problem_for, CompiledModel, QuboCompiler};
use std::sync::Arc;

/// Registry name of the local classical simulated annealing backend.
pub const NAME_SIMULATED_ANNEALING: &str = "dwave-sa";
/// Registry name of the local exhaustive classical backend.
pub const NAME_EXACT: &str = "dwave-exact";
/// Registry name of the Leap hybrid backend.
pub const NAME_HYBRID: &str = "dwave-hybrid";
/// Registry name of the quantum annealing backend.
pub const NAME_QPU: &str = "dwave-qpu";
/// Registry name of the local quantum annealing emulator backend.
pub const NAME_SIMULATED_QUANTUM_ANNEALING: &str = "dwave-sqa";

/// Reads a per-solve sampler seed from the problem metadata.
fn seed_hint(problem: &DecisionProblem) -> Option<u64> {
    problem
        .metadata
        .data
        .get(hints::SAMPLER_SEED)
        .and_then(|value| value.parse().ok())
}

/// Compiles a decision problem into the QUBO the bridge will sample.
fn compile(problem: &DecisionProblem, compiler: &QuboCompiler) -> Result<CompiledModel> {
    let model = optimization_problem_for(problem)?;
    Ok(compiler.compile(&model)?)
}

fn reject_if_too_large(
    model: &CompiledModel,
    capabilities: &SolverCapabilities,
    backend: &str,
) -> Result<()> {
    match capabilities.rejection_reason(model.bqm().num_variables()) {
        Some(reason) => Err(DWaveError::ProblemTooLarge {
            message: format!("{backend} refused the problem: {reason}"),
            cause: String::new(),
        }),
        None => Ok(()),
    }
}

/// Classical simulated annealing over an Ocean BQM.
///
/// This runs `dwave.samplers.SimulatedAnnealingSampler` on the local CPU. It is
/// classical metaheuristic search on a QUBO, not a simulation of a D-Wave QPU,
/// and it makes no quantum claim.
#[derive(Debug, Clone)]
pub struct DWaveSimulatedAnnealingBackend {
    bridge: Arc<DWaveBridge>,
    params: SimulatedAnnealingParams,
    compiler: QuboCompiler,
}

impl DWaveSimulatedAnnealingBackend {
    pub fn new(bridge: Arc<DWaveBridge>) -> Self {
        Self {
            bridge,
            params: SimulatedAnnealingParams::default(),
            compiler: QuboCompiler::default(),
        }
    }

    pub fn from_env() -> Self {
        Self::new(Arc::new(DWaveBridge::from_env()))
    }

    pub fn with_params(mut self, params: SimulatedAnnealingParams) -> Self {
        self.params = params;
        self
    }

    pub fn with_compiler(mut self, compiler: QuboCompiler) -> Self {
        self.compiler = compiler;
        self
    }

    async fn run(&self, problem: DecisionProblem) -> Result<SolverOutput> {
        // An explicitly configured seed wins; otherwise the caller can supply
        // one per solve, which is how a benchmark makes runs reproducible.
        let seed = self.params.seed.or_else(|| seed_hint(&problem));
        let model = compile(&problem, &self.compiler)?;
        let request = BridgeRequest::new(
            BridgeBackend::SimulatedAnnealing,
            model.problem().id.clone(),
            BridgeBqm::from(model.bqm()),
        )
        .with_parameter("num_reads", self.params.num_reads)
        .with_optional_parameter("num_sweeps", self.params.num_sweeps)
        .with_optional_parameter("seed", seed)
        .with_optional_parameter(
            "beta_range",
            self.params
                .beta_range
                .map(|(low, high)| serde_json::json!([low, high])),
        );

        let execution = self.bridge.execute(&request).await?;
        Ok(to_solver_output(
            NAME_SIMULATED_ANNEALING,
            SolverKind::Classical,
            &model,
            &execution,
        ))
    }
}

#[async_trait]
impl SolverBackend for DWaveSimulatedAnnealingBackend {
    fn name(&self) -> &'static str {
        NAME_SIMULATED_ANNEALING
    }

    fn kind(&self) -> SolverKind {
        // Simulated annealing runs on classical hardware even though it is
        // driven through a quantum vendor's SDK.
        SolverKind::Classical
    }

    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities {
            max_variables: None,
            supports_quadratic_models: true,
            supports_plan_output: true,
            remote: false,
            requires_credentials: false,
        }
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> CoreResult<SolverOutput> {
        Ok(self.run(problem).await?)
    }
}

/// Exhaustive classical search over the compiled QUBO.
///
/// Useful for validating a compilation and for checking heuristic results on
/// small instances. Guarded by a variable threshold because the search is
/// exponential.
#[derive(Debug, Clone)]
pub struct DWaveExactSolverBackend {
    bridge: Arc<DWaveBridge>,
    params: ExactParams,
    compiler: QuboCompiler,
}

impl DWaveExactSolverBackend {
    pub fn new(bridge: Arc<DWaveBridge>) -> Self {
        Self {
            bridge,
            params: ExactParams::default(),
            compiler: QuboCompiler::default(),
        }
    }

    pub fn from_env() -> Self {
        Self::new(Arc::new(DWaveBridge::from_env()))
    }

    pub fn with_params(mut self, params: ExactParams) -> Self {
        self.params = params;
        self
    }

    async fn run(&self, problem: DecisionProblem) -> Result<SolverOutput> {
        let model = compile(&problem, &self.compiler)?;
        reject_if_too_large(&model, &self.capabilities(), NAME_EXACT)?;

        let request = BridgeRequest::new(
            BridgeBackend::Exact,
            model.problem().id.clone(),
            BridgeBqm::from(model.bqm()),
        )
        .with_option("max_variables", self.params.max_variables);

        let execution = self.bridge.execute(&request).await?;
        Ok(to_solver_output(
            NAME_EXACT,
            SolverKind::Classical,
            &model,
            &execution,
        ))
    }
}

#[async_trait]
impl SolverBackend for DWaveExactSolverBackend {
    fn name(&self) -> &'static str {
        NAME_EXACT
    }

    fn kind(&self) -> SolverKind {
        SolverKind::Classical
    }

    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities {
            max_variables: Some(self.params.max_variables),
            supports_quadratic_models: true,
            supports_plan_output: true,
            remote: false,
            requires_credentials: false,
        }
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> CoreResult<SolverOutput> {
        Ok(self.run(problem).await?)
    }
}

/// D-Wave Leap hybrid solver.
///
/// Managed quantum/classical optimization. Requires Leap credentials, which
/// Ocean reads from the environment inside the bridge process.
#[derive(Debug, Clone)]
pub struct DWaveLeapHybridBackend {
    bridge: Arc<DWaveBridge>,
    leap: LeapConfig,
    params: HybridParams,
    compiler: QuboCompiler,
}

impl DWaveLeapHybridBackend {
    pub fn new(bridge: Arc<DWaveBridge>) -> Self {
        Self {
            bridge,
            leap: LeapConfig::default(),
            params: HybridParams::default(),
            compiler: QuboCompiler::default(),
        }
    }

    pub fn from_env() -> Self {
        Self::new(Arc::new(DWaveBridge::from_env())).with_leap(LeapConfig::from_env())
    }

    pub fn with_leap(mut self, leap: LeapConfig) -> Self {
        self.leap = leap;
        self
    }

    pub fn with_params(mut self, params: HybridParams) -> Self {
        self.params = params;
        self
    }

    async fn run(&self, problem: DecisionProblem) -> Result<SolverOutput> {
        let model = compile(&problem, &self.compiler)?;
        let request = BridgeRequest::new(
            BridgeBackend::Hybrid,
            model.problem().id.clone(),
            BridgeBqm::from(model.bqm()),
        )
        .with_optional_parameter("time_limit_s", self.params.time_limit_s)
        .with_optional_parameter("label", self.params.label.clone())
        .with_optional_option("solver", self.leap.solver.clone())
        .with_optional_option("region", self.leap.region.clone())
        .with_optional_option("endpoint", self.leap.endpoint.clone())
        .with_optional_option("profile", self.leap.profile.clone());

        let execution = self.bridge.execute(&request).await?;
        Ok(to_solver_output(
            NAME_HYBRID,
            SolverKind::QuantumHybrid,
            &model,
            &execution,
        ))
    }
}

#[async_trait]
impl SolverBackend for DWaveLeapHybridBackend {
    fn name(&self) -> &'static str {
        NAME_HYBRID
    }

    fn kind(&self) -> SolverKind {
        SolverKind::QuantumHybrid
    }

    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities {
            max_variables: None,
            supports_quadratic_models: true,
            supports_plan_output: true,
            remote: true,
            requires_credentials: true,
        }
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> CoreResult<SolverOutput> {
        Ok(self.run(problem).await?)
    }
}

/// Local emulation of quantum annealing dynamics.
///
/// This runs `dwave.samplers.PathIntegralAnnealingSampler` on the local CPU,
/// which emulates the path-integral (transverse-field) dynamics of a D-Wave
/// annealer. It is an emulator, not a QPU: it needs no credentials, no
/// network, and its results must never be described as real quantum results.
#[derive(Debug, Clone)]
pub struct DWaveSimulatedQuantumAnnealingBackend {
    bridge: Arc<DWaveBridge>,
    params: SimulatedQuantumAnnealingParams,
    compiler: QuboCompiler,
}

impl DWaveSimulatedQuantumAnnealingBackend {
    pub fn new(bridge: Arc<DWaveBridge>) -> Self {
        Self {
            bridge,
            params: SimulatedQuantumAnnealingParams::default(),
            compiler: QuboCompiler::default(),
        }
    }

    pub fn from_env() -> Self {
        Self::new(Arc::new(DWaveBridge::from_env()))
    }

    pub fn with_params(mut self, params: SimulatedQuantumAnnealingParams) -> Self {
        self.params = params;
        self
    }

    pub fn with_compiler(mut self, compiler: QuboCompiler) -> Self {
        self.compiler = compiler;
        self
    }

    async fn run(&self, problem: DecisionProblem) -> Result<SolverOutput> {
        // An explicitly configured seed wins; otherwise the caller can supply
        // one per solve, which is how a benchmark makes runs reproducible.
        let seed = self.params.seed.or_else(|| seed_hint(&problem));
        let model = compile(&problem, &self.compiler)?;
        let request = BridgeRequest::new(
            BridgeBackend::SimulatedQuantumAnnealing,
            model.problem().id.clone(),
            BridgeBqm::from(model.bqm()),
        )
        .with_parameter("num_reads", self.params.num_reads)
        .with_optional_parameter("num_sweeps", self.params.num_sweeps)
        .with_optional_parameter("seed", seed)
        .with_optional_parameter(
            "beta_range",
            self.params
                .beta_range
                .map(|(low, high)| serde_json::json!([low, high])),
        );

        let execution = self.bridge.execute(&request).await?;
        Ok(to_solver_output(
            NAME_SIMULATED_QUANTUM_ANNEALING,
            SolverKind::QuantumInspired,
            &model,
            &execution,
        ))
    }
}

#[async_trait]
impl SolverBackend for DWaveSimulatedQuantumAnnealingBackend {
    fn name(&self) -> &'static str {
        NAME_SIMULATED_QUANTUM_ANNEALING
    }

    fn kind(&self) -> SolverKind {
        // The sampler genuinely emulates quantum annealing dynamics, so it is
        // quantum-inspired — but it runs on a local CPU, so it is never
        // labelled as a quantum device.
        SolverKind::QuantumInspired
    }

    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities {
            max_variables: None,
            supports_quadratic_models: true,
            supports_plan_output: true,
            remote: false,
            requires_credentials: false,
        }
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> CoreResult<SolverOutput> {
        Ok(self.run(problem).await?)
    }
}

/// Quantum annealing on D-Wave hardware.
///
/// The problem is minor-embedded onto the QPU topology by
/// `EmbeddingComposite`. Not every problem can be embedded, and embedding
/// failures surface as a typed error rather than a silent fallback.
#[derive(Debug, Clone)]
pub struct DWaveQpuBackend {
    bridge: Arc<DWaveBridge>,
    leap: LeapConfig,
    params: QpuParams,
    compiler: QuboCompiler,
}

impl DWaveQpuBackend {
    pub fn new(bridge: Arc<DWaveBridge>) -> Self {
        Self {
            bridge,
            leap: LeapConfig::default(),
            params: QpuParams::default(),
            compiler: QuboCompiler::default(),
        }
    }

    pub fn from_env() -> Self {
        Self::new(Arc::new(DWaveBridge::from_env())).with_leap(LeapConfig::from_env())
    }

    pub fn with_leap(mut self, leap: LeapConfig) -> Self {
        self.leap = leap;
        self
    }

    pub fn with_params(mut self, params: QpuParams) -> Self {
        self.params = params;
        self
    }

    async fn run(&self, problem: DecisionProblem) -> Result<SolverOutput> {
        let model = compile(&problem, &self.compiler)?;
        reject_if_too_large(&model, &self.capabilities(), NAME_QPU)?;

        let request = BridgeRequest::new(
            BridgeBackend::Qpu,
            model.problem().id.clone(),
            BridgeBqm::from(model.bqm()),
        )
        .with_parameter("num_reads", self.params.num_reads)
        .with_optional_parameter("chain_strength", self.params.chain_strength)
        .with_optional_parameter("annealing_time_us", self.params.annealing_time_us)
        .with_optional_parameter("label", self.params.label.clone())
        .with_optional_option("solver", self.leap.solver.clone())
        .with_optional_option("region", self.leap.region.clone())
        .with_optional_option("endpoint", self.leap.endpoint.clone())
        .with_optional_option("profile", self.leap.profile.clone());

        let execution = self.bridge.execute(&request).await?;
        Ok(to_solver_output(
            NAME_QPU,
            SolverKind::QuantumAnnealing,
            &model,
            &execution,
        ))
    }
}

#[async_trait]
impl SolverBackend for DWaveQpuBackend {
    fn name(&self) -> &'static str {
        NAME_QPU
    }

    fn kind(&self) -> SolverKind {
        SolverKind::QuantumAnnealing
    }

    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities {
            max_variables: Some(self.params.max_variables),
            supports_quadratic_models: true,
            supports_plan_output: true,
            remote: true,
            requires_credentials: true,
        }
    }

    async fn solve(
        &self,
        problem: DecisionProblem,
        _context: SolverContext,
    ) -> CoreResult<SolverOutput> {
        Ok(self.run(problem).await?)
    }
}

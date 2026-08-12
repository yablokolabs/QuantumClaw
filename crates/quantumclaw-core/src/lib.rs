use async_trait::async_trait;
use quantumclaw_ir::optimization::OptimizationSolution;
use quantumclaw_ir::{DecisionProblem, ExecutionMetadata};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuantumClawError {
    pub message: String,
}

impl QuantumClawError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for QuantumClawError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for QuantumClawError {}

impl From<&str> for QuantumClawError {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for QuantumClawError {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

pub type Result<T> = std::result::Result<T, QuantumClawError>;

/// Well-known keys a caller can put in `DecisionProblem.metadata.data` to
/// steer a solver without changing the `SolverBackend` contract.
pub mod hints {
    /// Seed for stochastic samplers. Backends that support seeding use this
    /// when their own configuration does not already set one, which is what
    /// makes a benchmark repeatable.
    pub const SAMPLER_SEED: &str = "sampler.seed";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SolverKind {
    /// Runs entirely on classical hardware. Simulated annealing and exhaustive
    /// search belong here even when they are driven through a quantum vendor's
    /// SDK.
    Classical,
    QuantumInspired,
    /// Managed solvers that combine classical compute with quantum hardware,
    /// such as D-Wave Leap hybrid solvers.
    QuantumHybrid,
    /// Quantum annealing hardware, such as a D-Wave QPU.
    QuantumAnnealing,
    FutureQpu,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskType {
    Coding,
    Research,
    Workflow,
    Messaging,
    Browser,
    EnterpriseAssistant,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeploymentEnvironment {
    LocalCli,
    Server,
    Edge,
    Enterprise,
    Sandbox,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: String,
    pub description: String,
    pub task_type: TaskType,
    pub latency_budget_ms: Option<u64>,
    pub confidence_floor: f64,
    pub environment: DeploymentEnvironment,
    pub context: BTreeMap<String, String>,
}

impl AgentTask {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            id: "task-local".into(),
            description: description.into(),
            task_type: TaskType::Generic,
            latency_budget_ms: Some(30_000),
            confidence_floor: 0.55,
            environment: DeploymentEnvironment::LocalCli,
            context: BTreeMap::new(),
        }
    }

    pub fn with_task_type(mut self, task_type: TaskType) -> Self {
        self.task_type = task_type;
        self
    }

    pub fn with_latency_budget_ms(mut self, latency_budget_ms: u64) -> Self {
        self.latency_budget_ms = Some(latency_budget_ms);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolverContext {
    pub task_type: TaskType,
    pub latency_budget_ms: Option<u64>,
    pub confidence_floor: f64,
    pub environment: DeploymentEnvironment,
}

impl SolverContext {
    pub fn from_task(task: &AgentTask) -> Self {
        Self {
            task_type: task.task_type,
            latency_budget_ms: task.latency_budget_ms,
            confidence_floor: task.confidence_floor,
            environment: task.environment,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolverPlanStep {
    pub id: String,
    pub action_id: Option<String>,
    pub title: String,
    pub tool_hint: Option<String>,
    pub rationale: String,
    pub expected_utility: f64,
    pub risk: f64,
}

impl SolverPlanStep {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            action_id: None,
            title: title.into(),
            tool_hint: None,
            rationale: String::new(),
            expected_utility: 0.5,
            risk: 0.1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolverScore {
    pub utility: f64,
    pub confidence: f64,
    pub cost_estimate: f64,
    pub risk: f64,
}

impl Default for SolverScore {
    fn default() -> Self {
        Self {
            utility: 0.0,
            confidence: 0.5,
            cost_estimate: 0.0,
            risk: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendTelemetry {
    pub backend: String,
    pub backend_kind: SolverKind,
    pub latency_ms: u64,
    pub cost_estimate: f64,
    pub confidence: f64,
    pub notes: Vec<String>,
    /// Provider that executed the solve, for example `dwave`. Absent for
    /// backends that run in-process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Provider-specific run metadata. Kept as free-form JSON so no vendor
    /// type leaks into the core domain model. Fields that a provider cannot
    /// measure are simply absent rather than fabricated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

impl BackendTelemetry {
    pub fn new(backend: impl Into<String>, backend_kind: SolverKind) -> Self {
        Self {
            backend: backend.into(),
            backend_kind,
            latency_ms: 0,
            cost_estimate: 0.0,
            confidence: 0.5,
            notes: Vec::new(),
            provider: None,
            provider_metadata: None,
        }
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    pub fn with_provider_metadata(mut self, metadata: Value) -> Self {
        self.provider_metadata = Some(metadata);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolverOutput {
    pub backend: String,
    pub backend_kind: SolverKind,
    pub steps: Vec<SolverPlanStep>,
    pub score: SolverScore,
    pub rationale: String,
    pub telemetry: BackendTelemetry,
    /// Normalized combinatorial result for backends that optimize over binary
    /// decision variables. Plan-shaped backends leave this empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solution: Option<OptimizationSolution>,
}

/// What a solver backend can accept, so callers can check before submitting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolverCapabilities {
    /// Largest number of binary variables the backend will accept, when it
    /// declares a limit.
    pub max_variables: Option<usize>,
    /// Whether the backend optimizes over an explicit binary quadratic model.
    pub supports_quadratic_models: bool,
    /// Whether the backend can produce a plan from candidate actions alone.
    pub supports_plan_output: bool,
    /// Whether the backend calls a remote service.
    pub remote: bool,
    /// Whether the backend needs credentials before it can run.
    pub requires_credentials: bool,
}

impl Default for SolverCapabilities {
    fn default() -> Self {
        Self {
            max_variables: None,
            supports_quadratic_models: false,
            supports_plan_output: true,
            remote: false,
            requires_credentials: false,
        }
    }
}

impl SolverCapabilities {
    /// Reports why a problem of this size cannot be submitted, if it cannot.
    pub fn rejection_reason(&self, variables: usize) -> Option<String> {
        match self.max_variables {
            Some(limit) if variables > limit => Some(format!(
                "problem has {variables} variables but this backend accepts at most {limit}"
            )),
            _ => None,
        }
    }
}

#[async_trait]
pub trait Planner: Send + Sync {
    type Request: Send + 'static;
    type Response: Send + 'static;

    async fn plan(&self, request: Self::Request) -> Result<Self::Response>;
}

#[async_trait]
pub trait ProblemEncoder: Send + Sync {
    async fn encode(&self, task: &AgentTask) -> Result<DecisionProblem>;
}

#[async_trait]
pub trait SolverBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn kind(&self) -> SolverKind;
    async fn solve(&self, problem: DecisionProblem, context: SolverContext)
        -> Result<SolverOutput>;

    /// What this backend accepts. The default suits in-process planners.
    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities::default()
    }
}

/// Name-keyed lookup of solver backends.
///
/// Backends register under their own name and optionally under short aliases,
/// which is how `--backend dwave-sa` reaches an implementation without the
/// caller knowing which crate provides it.
#[derive(Default, Clone)]
pub struct SolverRegistry {
    backends: BTreeMap<String, Arc<dyn SolverBackend>>,
}

impl SolverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a backend under its own name.
    pub fn register(&mut self, backend: Arc<dyn SolverBackend>) -> &mut Self {
        self.backends.insert(backend.name().to_string(), backend);
        self
    }

    /// Registers a backend under an additional short name.
    pub fn register_as(
        &mut self,
        alias: impl Into<String>,
        backend: Arc<dyn SolverBackend>,
    ) -> &mut Self {
        self.backends.insert(alias.into(), backend);
        self
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn SolverBackend>> {
        self.backends.get(name).cloned()
    }

    /// Resolves a backend or explains what is available.
    pub fn require(&self, name: &str) -> Result<Arc<dyn SolverBackend>> {
        self.get(name).ok_or_else(|| {
            QuantumClawError::new(format!(
                "unknown solver backend '{name}'; available backends: {}",
                self.names().join(", ")
            ))
        })
    }

    pub fn names(&self) -> Vec<String> {
        self.backends.keys().cloned().collect()
    }

    pub fn backends(&self) -> Vec<Arc<dyn SolverBackend>> {
        self.backends.values().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }
}

impl std::fmt::Debug for SolverRegistry {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SolverRegistry")
            .field("backends", &self.names())
            .finish()
    }
}

#[async_trait]
pub trait PlanDecoder: Send + Sync {
    type Plan: Send + 'static;

    async fn decode(&self, output: SolverOutput, metadata: ExecutionMetadata)
        -> Result<Self::Plan>;
}

#[async_trait]
pub trait MemoryStore: Send + Sync {
    type Record: Clone + Send + Sync + 'static;

    async fn put(&self, record: Self::Record) -> Result<()>;
    async fn query(&self, query: &str, limit: usize) -> Result<Vec<Self::Record>>;
}

#[async_trait]
pub trait SkillStore: Send + Sync {
    type Skill: Clone + Send + Sync + 'static;

    async fn save_skill(&self, skill: Self::Skill) -> Result<()>;
    async fn find_skills(&self, query: &str, limit: usize) -> Result<Vec<Self::Skill>>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreToolCall {
    pub tool_name: String,
    pub action: String,
    pub input: Value,
    pub metadata: BTreeMap<String, String>,
}

impl CoreToolCall {
    pub fn new(tool_name: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            action: action.into(),
            input: Value::Null,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreToolResult {
    pub success: bool,
    pub output: Value,
    pub metadata: BTreeMap<String, String>,
}

impl CoreToolResult {
    pub fn simulated(message: impl Into<String>) -> Self {
        Self {
            success: true,
            output: Value::String(message.into()),
            metadata: BTreeMap::new(),
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn call(&self, call: CoreToolCall) -> Result<CoreToolResult>;
}

#[async_trait]
pub trait ToolRegistry: Send + Sync {
    async fn register(&self, tool: Arc<dyn Tool>) -> Result<()>;
    async fn get(&self, name: &str) -> Option<Arc<dyn Tool>>;
    async fn list(&self) -> Vec<String>;
}

#[async_trait]
pub trait PolicyEngine: Send + Sync {
    type Plan: Send + Sync;
    type Decision: Send + Sync;

    async fn evaluate_plan(&self, plan: &Self::Plan) -> Result<Self::Decision>;
}

#[async_trait]
pub trait Observer: Send + Sync {
    async fn observe(&self, event: Value) -> Result<()>;
}

#[async_trait]
pub trait SubagentRegistry: Send + Sync {
    async fn register_subagent(&self, id: String, capability: String) -> Result<()>;
    async fn list_subagents(&self) -> Result<Vec<(String, String)>>;
}

#[async_trait]
pub trait RuntimeAdapter: Send + Sync {
    async fn start(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}

#[async_trait]
pub trait AgentRuntime: Send + Sync {
    type Request: Send + 'static;
    type Response: Send + 'static;

    async fn handle(&self, request: Self::Request) -> Result<Self::Response>;
}

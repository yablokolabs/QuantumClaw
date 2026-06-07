use crate::quantumclaw_ir::{DecisionProblem, ExecutionMetadata};
use async_trait::async_trait;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SolverKind {
    Classical,
    QuantumInspired,
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
        }
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

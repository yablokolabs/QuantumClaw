use crate::quantumclaw_core::{
    AgentRuntime as CoreAgentRuntime, AgentTask, CoreToolCall, Result, SolverBackend,
    SubagentRegistry as CoreSubagentRegistry, TaskType, ToolRegistry,
};
pub use crate::quantumclaw_core::{AgentRuntime, RuntimeAdapter, SubagentRegistry};
use crate::quantumclaw_memory::{InMemoryProceduralMemory, ProceduralMemory, StoredProcedure};
use crate::quantumclaw_observability::{
    ExecutionTrace, InMemoryObserver, PlannerComparisonEvent, TraceEvent,
};
use crate::quantumclaw_planner::{HybridPlanner, Plan, PlannerMode, PlannerRequest};
use crate::quantumclaw_policy::{DeterministicPolicyEngine, PolicyDecision};
use crate::quantumclaw_skills::{Skill, SkillExecutionRecord, SkillLearningPipeline};
use crate::quantumclaw_tools::InMemoryToolRegistry;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use zeroclaw::memory::{
    Memory as ZeroClawMemory, MemoryCategory as ZeroClawMemoryCategory,
    MemoryEntry as ZeroClawMemoryEntry,
};
use zeroclaw::runtime::{
    NativeRuntime as ZeroClawNativeRuntime, RuntimeAdapter as ZeroClawRuntimeAdapter,
};
use zeroclaw::Config as ZeroClawConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZeroClawExecutionContext {
    pub runtime_name: String,
    pub has_shell_access: bool,
    pub has_filesystem_access: bool,
    pub supports_long_running: bool,
    pub storage_path: PathBuf,
    pub tool_substrate: String,
    pub memory_backend: String,
}

#[derive(Clone)]
pub struct ZeroClawBase {
    config: Arc<ZeroClawConfig>,
    runtime: Arc<dyn ZeroClawRuntimeAdapter>,
    memory: Arc<dyn ZeroClawMemory>,
    tool_substrate: &'static str,
}

impl ZeroClawBase {
    pub fn native(memory: InMemoryProceduralMemory) -> Self {
        Self {
            config: Arc::new(ZeroClawConfig::default()),
            runtime: Arc::new(ZeroClawNativeRuntime::new()),
            memory: Arc::new(ZeroClawProceduralMemory::new(memory)),
            tool_substrate: "zeroclaw::tools::Tool",
        }
    }

    pub fn config(&self) -> Arc<ZeroClawConfig> {
        self.config.clone()
    }

    pub fn runtime_adapter(&self) -> &dyn ZeroClawRuntimeAdapter {
        self.runtime.as_ref()
    }

    pub fn memory(&self) -> Arc<dyn ZeroClawMemory> {
        self.memory.clone()
    }

    pub fn memory_backend_name(&self) -> &str {
        self.memory.name()
    }

    pub fn execution_context(&self) -> ZeroClawExecutionContext {
        ZeroClawExecutionContext {
            runtime_name: self.runtime.name().into(),
            has_shell_access: self.runtime.has_shell_access(),
            has_filesystem_access: self.runtime.has_filesystem_access(),
            supports_long_running: self.runtime.supports_long_running(),
            storage_path: self.runtime.storage_path(),
            tool_substrate: self.tool_substrate.into(),
            memory_backend: self.memory.name().into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ZeroClawProceduralMemory {
    inner: InMemoryProceduralMemory,
}

impl ZeroClawProceduralMemory {
    pub fn new(inner: InMemoryProceduralMemory) -> Self {
        Self { inner }
    }

    fn entry_from_procedure(procedure: StoredProcedure) -> ZeroClawMemoryEntry {
        let category = procedure
            .metadata
            .get("zeroclaw_category")
            .map(|category| category_from_label(category))
            .unwrap_or(ZeroClawMemoryCategory::Core);
        let session_id = procedure.metadata.get("zeroclaw_session_id").cloned();

        ZeroClawMemoryEntry {
            id: procedure.id.clone(),
            key: procedure.id,
            content: procedure.summary,
            category,
            timestamp: "1970-01-01T00:00:00Z".into(),
            session_id,
            score: None,
        }
    }
}

#[async_trait]
impl ZeroClawMemory for ZeroClawProceduralMemory {
    fn name(&self) -> &str {
        "quantumclaw-procedural-memory"
    }

    async fn store(
        &self,
        key: &str,
        content: &str,
        category: ZeroClawMemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let keywords = content
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|token| token.len() >= 3)
            .take(16)
            .map(str::to_ascii_lowercase);
        let mut procedure = StoredProcedure::new(key, content, keywords);
        procedure
            .metadata
            .insert("zeroclaw_category".into(), category.to_string());
        if let Some(session_id) = session_id {
            procedure
                .metadata
                .insert("zeroclaw_session_id".into(), session_id.into());
        }
        self.inner
            .store_procedure(procedure)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<ZeroClawMemoryEntry>> {
        let procedures = self
            .inner
            .retrieve_similar(query, limit)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(procedures
            .into_iter()
            .filter(|procedure| {
                session_id.is_none_or(|session_id| {
                    procedure
                        .metadata
                        .get("zeroclaw_session_id")
                        .is_some_and(|stored| stored == session_id)
                })
            })
            .map(Self::entry_from_procedure)
            .collect())
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<ZeroClawMemoryEntry>> {
        Ok(self
            .inner
            .all_procedures()
            .into_iter()
            .find(|procedure| procedure.id == key)
            .map(Self::entry_from_procedure))
    }

    async fn list(
        &self,
        category: Option<&ZeroClawMemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<ZeroClawMemoryEntry>> {
        Ok(self
            .inner
            .all_procedures()
            .into_iter()
            .filter(|procedure| {
                let category_matches = category.is_none_or(|expected| {
                    procedure
                        .metadata
                        .get("zeroclaw_category")
                        .is_some_and(|stored| stored == &expected.to_string())
                });
                let session_matches = session_id.is_none_or(|expected| {
                    procedure
                        .metadata
                        .get("zeroclaw_session_id")
                        .is_some_and(|stored| stored == expected)
                });
                category_matches && session_matches
            })
            .map(Self::entry_from_procedure)
            .collect())
    }

    async fn forget(&self, key: &str) -> anyhow::Result<bool> {
        Ok(self.inner.forget_procedure(key))
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(self.inner.count_procedures())
    }

    async fn health_check(&self) -> bool {
        true
    }
}

fn category_from_label(label: &str) -> ZeroClawMemoryCategory {
    match label {
        "core" => ZeroClawMemoryCategory::Core,
        "daily" => ZeroClawMemoryCategory::Daily,
        "conversation" => ZeroClawMemoryCategory::Conversation,
        other => ZeroClawMemoryCategory::Custom(other.into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub id: String,
    pub body: String,
    pub variables: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRequest {
    pub prompt: String,
    pub model_hint: Option<String>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderResponse {
    pub text: String,
    pub provider: String,
    pub model: String,
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    async fn complete(&self, request: ProviderRequest) -> Result<ProviderResponse>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u8,
    pub backoff_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 2,
            backoff_ms: 250,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentLifecycle {
    Created,
    Planning,
    PolicyCheck,
    Executing,
    Learning,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSession {
    pub id: String,
    pub user_task: String,
    pub lifecycle: AgentLifecycle,
    pub channel: Option<String>,
}

impl RuntimeSession {
    pub fn new(user_task: impl Into<String>) -> Self {
        Self {
            id: "session-local".into(),
            user_task: user_task.into(),
            lifecycle: AgentLifecycle::Created,
            channel: None,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct MessageRouter {
    routes: Arc<RwLock<HashMap<String, String>>>,
}

impl MessageRouter {
    pub fn add_route(&self, channel: impl Into<String>, adapter: impl Into<String>) {
        self.routes
            .write()
            .expect("message router lock")
            .insert(channel.into(), adapter.into());
    }
}

#[async_trait]
pub trait Channel: Send + Sync {
    async fn send(&self, session: &RuntimeSession, message: &str) -> Result<()>;
}

#[derive(Debug, Default, Clone)]
pub struct InMemorySubagentRegistry {
    subagents: Arc<RwLock<Vec<(String, String)>>>,
}

#[async_trait]
impl CoreSubagentRegistry for InMemorySubagentRegistry {
    async fn register_subagent(&self, id: String, capability: String) -> Result<()> {
        self.subagents
            .write()
            .expect("subagent registry lock")
            .push((id, capability));
        Ok(())
    }

    async fn list_subagents(&self) -> Result<Vec<(String, String)>> {
        Ok(self
            .subagents
            .read()
            .expect("subagent registry lock")
            .clone())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulatedExecution {
    pub resolved_tools: Vec<String>,
    pub tool_results: Vec<String>,
    pub rollback_available: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeExecutionReport {
    pub session: RuntimeSession,
    pub retrieved_procedures: Vec<StoredProcedure>,
    pub plan: Plan,
    pub policy_decision: PolicyDecision,
    pub execution: SimulatedExecution,
    pub telemetry: ExecutionTrace,
    pub learned_skill: Skill,
    pub zeroclaw: ZeroClawExecutionContext,
}

#[derive(Clone)]
pub struct QuantumClawRuntime {
    pub zeroclaw: ZeroClawBase,
    pub planner: HybridPlanner,
    pub backends: Vec<Arc<dyn SolverBackend>>,
    pub memory: InMemoryProceduralMemory,
    pub policy: DeterministicPolicyEngine,
    pub tools: InMemoryToolRegistry,
    pub observer: InMemoryObserver,
    pub skill_pipeline: SkillLearningPipeline<InMemoryProceduralMemory>,
}

impl QuantumClawRuntime {
    pub fn new(
        backends: Vec<Arc<dyn SolverBackend>>,
        memory: InMemoryProceduralMemory,
        tools: InMemoryToolRegistry,
        policy: DeterministicPolicyEngine,
        observer: InMemoryObserver,
    ) -> Self {
        let zeroclaw = ZeroClawBase::native(memory.clone());
        Self::new_with_zeroclaw_base(backends, memory, tools, policy, observer, zeroclaw)
    }

    pub fn new_with_zeroclaw_base(
        backends: Vec<Arc<dyn SolverBackend>>,
        memory: InMemoryProceduralMemory,
        tools: InMemoryToolRegistry,
        policy: DeterministicPolicyEngine,
        observer: InMemoryObserver,
        zeroclaw: ZeroClawBase,
    ) -> Self {
        let skill_pipeline = SkillLearningPipeline::new(memory.clone());
        Self {
            zeroclaw,
            planner: HybridPlanner::default(),
            backends,
            memory,
            policy,
            tools,
            observer,
            skill_pipeline,
        }
    }

    pub async fn handle_user_task(
        &self,
        task: impl Into<String>,
    ) -> Result<RuntimeExecutionReport> {
        let task = task.into();
        let mut session = RuntimeSession::new(task.clone());
        session.lifecycle = AgentLifecycle::Planning;
        self.observer.record_trace(TraceEvent::new(
            "session.created",
            "runtime session created",
        ));
        let zeroclaw_context = self.zeroclaw.execution_context();
        self.observer.record_trace(TraceEvent::new(
            "zeroclaw.base.ready",
            format!(
                "using ZeroClaw {} runtime with {} tool substrate",
                zeroclaw_context.runtime_name, zeroclaw_context.tool_substrate
            ),
        ));

        let retrieved = self.memory.retrieve_similar(&task, 5).await?;
        let retrieved_summaries = retrieved
            .iter()
            .map(|procedure| procedure.summary.clone())
            .collect::<Vec<_>>();

        let mut request =
            PlannerRequest::new(AgentTask::new(task.clone()).with_task_type(TaskType::Coding))
                .with_mode(PlannerMode::Auto)
                .with_retrieved_skills(retrieved_summaries);
        for backend in &self.backends {
            request = request.with_backend(backend.clone());
        }
        let response = self.planner.plan(request).await?;
        let plan = response.primary_plan().clone();

        let mut trace = TraceEvent::new(
            "planner.selected",
            "planner selected backend and produced scored plan",
        );
        trace.selected_backend = Some(response.telemetry.selected_backend.clone());
        trace.plan_score = Some(response.telemetry.plan_score.utility);
        trace.latency_ms = Some(response.telemetry.latency_ms);
        trace.cost_estimate = Some(response.telemetry.plan_score.cost_estimate);
        trace.confidence = Some(response.telemetry.plan_score.confidence);
        self.observer.record_trace(trace);
        if let Some(comparison) = response.telemetry.shadow_comparison {
            self.observer.record_comparison(PlannerComparisonEvent {
                primary_backend: comparison.primary_backend,
                primary_backend_kind: comparison.primary_backend_kind,
                shadow_backend: comparison.shadow_backend,
                shadow_backend_kind: comparison.shadow_backend_kind,
                primary_score: comparison.primary_score.utility,
                shadow_score: comparison.shadow_score.utility,
                latency_ms: comparison.latency_ms,
            });
        }

        session.lifecycle = AgentLifecycle::PolicyCheck;
        let decision = self.policy.evaluate_plan(&plan).await?;
        let mut policy_trace = TraceEvent::new(
            "policy.decision",
            "deterministic policy evaluated proposed plan",
        );
        policy_trace.policy_decision = Some(
            if decision.allowed {
                "allowed"
            } else {
                "denied"
            }
            .into(),
        );
        self.observer.record_trace(policy_trace);
        if !decision.allowed {
            session.lifecycle = AgentLifecycle::Failed;
            return Err(format!("policy rejected plan: {}", decision.reasons.join("; ")).into());
        }

        session.lifecycle = AgentLifecycle::Executing;
        let mut resolved_tools = Vec::new();
        let mut tool_results = Vec::new();
        for step in &plan.steps {
            if let Some(tool) = self.tools.get(&step.tool_name).await {
                let result = tool
                    .call(CoreToolCall::new(
                        step.tool_name.clone(),
                        step.title.clone(),
                    ))
                    .await?;
                resolved_tools.push(step.tool_name.clone());
                tool_results.push(result.output.to_string());
            } else {
                tool_results.push(format!("missing tool stub for {}", step.tool_name));
            }
        }

        session.lifecycle = AgentLifecycle::Learning;
        let learned_skill = self.skill_pipeline.learn_from_success(SkillExecutionRecord {
            task: task.clone(),
            plan_summary: plan.rationale.summary.clone(),
            outcome: "simulated execution completed under deterministic policy".into(),
            why_it_worked: "the plan used small reversible steps, validation gates, and policy-controlled tools".into(),
            tags: vec!["coding".into(), "rust".into(), "refactor".into(), "planning".into()],
        }).await?;

        session.lifecycle = AgentLifecycle::Completed;
        self.observer.record_trace(TraceEvent::new(
            "runtime.completed",
            "execution completed and procedural learning captured",
        ));

        Ok(RuntimeExecutionReport {
            session,
            retrieved_procedures: retrieved,
            plan,
            policy_decision: decision,
            execution: SimulatedExecution {
                resolved_tools,
                tool_results,
                rollback_available: true,
            },
            telemetry: self.observer.execution_trace(),
            learned_skill,
            zeroclaw: zeroclaw_context,
        })
    }
}

#[async_trait]
impl CoreAgentRuntime for QuantumClawRuntime {
    type Request = String;
    type Response = RuntimeExecutionReport;

    async fn handle(&self, request: Self::Request) -> Result<Self::Response> {
        self.handle_user_task(request).await
    }
}

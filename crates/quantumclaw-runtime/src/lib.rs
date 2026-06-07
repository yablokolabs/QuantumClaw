use async_trait::async_trait;
use quantumclaw_core::{
    AgentRuntime as CoreAgentRuntime, AgentTask, CoreToolCall, Result, SolverBackend,
    SubagentRegistry as CoreSubagentRegistry, TaskType, ToolRegistry,
};
pub use quantumclaw_core::{AgentRuntime, RuntimeAdapter, SubagentRegistry};
use quantumclaw_memory::{InMemoryProceduralMemory, ProceduralMemory, StoredProcedure};
use quantumclaw_observability::{
    ExecutionTrace, InMemoryObserver, PlannerComparisonEvent, TraceEvent,
};
use quantumclaw_planner::{HybridPlanner, Plan, PlannerMode, PlannerRequest};
use quantumclaw_policy::{DeterministicPolicyEngine, PolicyDecision};
use quantumclaw_skills::{Skill, SkillExecutionRecord, SkillLearningPipeline};
use quantumclaw_tools::InMemoryToolRegistry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

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
}

#[derive(Clone)]
pub struct QuantumClawRuntime {
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
        let skill_pipeline = SkillLearningPipeline::new(memory.clone());
        Self {
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

use async_trait::async_trait;
use quantumclaw_core::{
    AgentTask, PlanDecoder, ProblemEncoder, QuantumClawError, Result, SolverBackend, SolverContext,
    SolverKind, SolverOutput,
};
use quantumclaw_ir::optimization::ObjectiveSense;
use quantumclaw_ir::{DecisionProblem, ExecutionMetadata};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDecomposition {
    pub task: String,
    pub subtasks: Vec<String>,
    pub dependency_notes: Vec<String>,
}

#[async_trait]
pub trait TaskDecomposer: Send + Sync {
    async fn decompose(&self, task: &AgentTask) -> Result<TaskDecomposition>;
}

#[derive(Debug, Default, Clone)]
pub struct SimpleTaskDecomposer;

#[async_trait]
impl TaskDecomposer for SimpleTaskDecomposer {
    async fn decompose(&self, task: &AgentTask) -> Result<TaskDecomposition> {
        let problem = DecisionProblem::for_task(task.description.clone());
        Ok(TaskDecomposition {
            task: task.description.clone(),
            subtasks: problem
                .subtasks
                .into_iter()
                .map(|subtask| subtask.description)
                .collect(),
            dependency_notes: problem
                .dependencies
                .into_iter()
                .map(|dependency| dependency.reason)
                .collect(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlannerMode {
    Reactive,
    Deliberative,
    Hybrid,
    ClassicalOnly,
    QuantumInspiredPreferred,
    Auto,
    ShadowCompare,
}

pub struct PlannerRequest {
    pub task: AgentTask,
    pub problem: Option<DecisionProblem>,
    pub mode: PlannerMode,
    pub backends: Vec<Arc<dyn SolverBackend>>,
    pub selection_policy: BackendSelectionPolicy,
    pub shadow_backend: Option<Arc<dyn SolverBackend>>,
    pub retrieved_skills: Vec<String>,
}

impl PlannerRequest {
    pub fn new(task: AgentTask) -> Self {
        Self {
            task,
            problem: None,
            mode: PlannerMode::Auto,
            backends: Vec::new(),
            selection_policy: BackendSelectionPolicy::default(),
            shadow_backend: None,
            retrieved_skills: Vec::new(),
        }
    }

    pub fn with_problem(mut self, problem: DecisionProblem) -> Self {
        self.problem = Some(problem);
        self
    }

    pub fn with_mode(mut self, mode: PlannerMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_backend(mut self, backend: Arc<dyn SolverBackend>) -> Self {
        self.backends.push(backend);
        self
    }

    pub fn with_selection_policy(mut self, selection_policy: BackendSelectionPolicy) -> Self {
        self.selection_policy = selection_policy;
        self
    }

    pub fn with_shadow_backend(mut self, backend: Arc<dyn SolverBackend>) -> Self {
        self.shadow_backend = Some(backend);
        self
    }

    pub fn with_retrieved_skills(mut self, skills: Vec<String>) -> Self {
        self.retrieved_skills = skills;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannerResponse {
    pub plans: Vec<Plan>,
    pub rationale: PlannerRationale,
    pub telemetry: PlannerTelemetry,
}

impl PlannerResponse {
    pub fn primary_plan(&self) -> &Plan {
        self.plans
            .first()
            .expect("planner response must contain at least one plan")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub backend: String,
    pub backend_kind: SolverKind,
    pub steps: Vec<PlanStep>,
    pub score: PlanScore,
    pub rationale: PlannerRationale,
    pub metadata: ExecutionMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    pub tool_name: String,
    pub action_id: Option<String>,
    pub rationale: String,
    pub risk_level: String,
}

impl PlanStep {
    pub fn new(title: impl Into<String>, tool_name: impl Into<String>) -> Self {
        let title = title.into();
        Self {
            id: title.to_lowercase().replace(' ', "-"),
            title,
            tool_name: tool_name.into(),
            action_id: None,
            rationale: String::new(),
            risk_level: "low".into(),
        }
    }

    pub fn with_risk(mut self, risk: impl ToString) -> Self {
        self.risk_level = risk.to_string();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanScore {
    pub utility: f64,
    pub confidence: f64,
    pub cost_estimate: f64,
    pub risk: f64,
}

impl Default for PlanScore {
    fn default() -> Self {
        Self {
            utility: 0.0,
            confidence: 0.5,
            cost_estimate: 0.0,
            risk: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerRationale {
    pub summary: String,
    pub selected_backend_reason: String,
    pub rejected_backend_reasons: Vec<String>,
}

impl PlannerRationale {
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            selected_backend_reason: String::new(),
            rejected_backend_reasons: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendSelectionPolicy {
    pub preferred_kind: Option<SolverKind>,
    /// Exact backend name to use, such as `dwave-sa`. Takes precedence over
    /// every kind-based preference, and fails loudly when it is unavailable so
    /// an explicit request is never silently downgraded.
    #[serde(default)]
    pub preferred_backend: Option<String>,
    pub allow_shadow: bool,
    pub max_latency_ms: Option<u64>,
    pub confidence_floor: f64,
}

impl BackendSelectionPolicy {
    pub fn prefer(kind: SolverKind) -> Self {
        Self {
            preferred_kind: Some(kind),
            ..Self::default()
        }
    }

    pub fn prefer_backend(name: impl Into<String>) -> Self {
        Self {
            preferred_backend: Some(name.into()),
            ..Self::default()
        }
    }

    pub fn with_preferred_backend(mut self, name: impl Into<String>) -> Self {
        self.preferred_backend = Some(name.into());
        self
    }

    pub fn with_shadow(mut self, allow_shadow: bool) -> Self {
        self.allow_shadow = allow_shadow;
        self
    }
}

impl Default for BackendSelectionPolicy {
    fn default() -> Self {
        Self {
            preferred_kind: None,
            preferred_backend: None,
            allow_shadow: false,
            max_latency_ms: Some(30_000),
            confidence_floor: 0.55,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannerTelemetry {
    pub selected_backend: String,
    pub selected_backend_kind: SolverKind,
    pub rejected_backends: Vec<String>,
    pub plan_score: PlanScore,
    pub latency_ms: u64,
    pub shadow_comparison: Option<ShadowComparison>,
    pub backend_telemetry: Vec<quantumclaw_core::BackendTelemetry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowComparison {
    pub primary_backend: String,
    pub primary_backend_kind: SolverKind,
    pub shadow_backend: String,
    pub shadow_backend_kind: SolverKind,
    pub primary_score: PlanScore,
    pub shadow_score: PlanScore,
    /// Wall time of the shadow run.
    pub latency_ms: u64,
    /// Wall time of the primary run.
    #[serde(default)]
    pub primary_latency_ms: u64,
    /// Objective-level comparison, present when both backends returned an
    /// optimization result.
    #[serde(default)]
    pub optimization: Option<OptimizationComparison>,
}

/// Which solver produced the better result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparisonVerdict {
    Primary,
    Shadow,
    Tie,
    /// The two results cannot be compared, for example because they optimize
    /// different objectives or neither is feasible.
    NotComparable,
}

/// Objective, feasibility, and runtime comparison of two solver results.
///
/// This is what a shadow deployment actually needs: not which backend is more
/// interesting, but whether it produced a feasible answer and how its objective
/// compares against the solver currently making decisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationComparison {
    pub sense: ObjectiveSense,
    pub primary_objective: f64,
    pub shadow_objective: f64,
    /// Improvement of the shadow result over the primary, always oriented so
    /// that a positive number means the shadow did better.
    pub objective_delta: f64,
    /// `objective_delta` relative to the primary objective, when that is
    /// non-zero. This is the optimality gap when the primary is a proven
    /// optimum.
    pub relative_gap: Option<f64>,
    pub primary_feasible: bool,
    pub shadow_feasible: bool,
    pub primary_violations: usize,
    pub shadow_violations: usize,
    /// In-solver runtime reported by the provider, when it measures one.
    pub primary_solver_runtime_ms: Option<f64>,
    pub shadow_solver_runtime_ms: Option<f64>,
    pub verdict: ComparisonVerdict,
    /// Domain metadata, for example logistics KPIs supplied by a brain.
    #[serde(default)]
    pub kpis: BTreeMap<String, f64>,
}

impl OptimizationComparison {
    /// Builds a comparison from two solver outputs.
    ///
    /// An infeasible result never wins: a better objective that breaks a hard
    /// constraint is not a better answer.
    pub fn between(primary: &SolverOutput, shadow: &SolverOutput) -> Option<Self> {
        let left = primary.solution.as_ref()?;
        let right = shadow.solution.as_ref()?;

        let improvement = match left.sense {
            ObjectiveSense::Minimize => left.objective_value - right.objective_value,
            ObjectiveSense::Maximize => right.objective_value - left.objective_value,
        };
        let relative_gap = if left.objective_value.abs() > f64::EPSILON {
            Some(improvement / left.objective_value.abs())
        } else {
            None
        };

        let verdict = match (left.feasible, right.feasible) {
            (true, false) => ComparisonVerdict::Primary,
            (false, true) => ComparisonVerdict::Shadow,
            (false, false) => ComparisonVerdict::NotComparable,
            (true, true) if improvement.abs() <= f64::EPSILON => ComparisonVerdict::Tie,
            (true, true) if improvement > 0.0 => ComparisonVerdict::Shadow,
            (true, true) => ComparisonVerdict::Primary,
        };

        Some(Self {
            sense: left.sense,
            primary_objective: left.objective_value,
            shadow_objective: right.objective_value,
            objective_delta: improvement,
            relative_gap,
            primary_feasible: left.feasible,
            shadow_feasible: right.feasible,
            primary_violations: left.violations.len(),
            shadow_violations: right.violations.len(),
            primary_solver_runtime_ms: solver_runtime_ms(&primary.telemetry),
            shadow_solver_runtime_ms: solver_runtime_ms(&shadow.telemetry),
            verdict,
            kpis: BTreeMap::new(),
        })
    }

    pub fn with_kpi(mut self, name: impl Into<String>, value: f64) -> Self {
        self.kpis.insert(name.into(), value);
        self
    }
}

/// Providers that measure time spent inside the solver itself report it under
/// `solver_runtime_ms` in their provider metadata. Wall time already lives in
/// [`quantumclaw_core::BackendTelemetry::latency_ms`].
fn solver_runtime_ms(telemetry: &quantumclaw_core::BackendTelemetry) -> Option<f64> {
    telemetry
        .provider_metadata
        .as_ref()?
        .get("solver_runtime_ms")?
        .as_f64()
}

#[derive(Debug, Default, Clone)]
pub struct DefaultProblemEncoder;

#[async_trait]
impl ProblemEncoder for DefaultProblemEncoder {
    async fn encode(&self, task: &AgentTask) -> Result<DecisionProblem> {
        Ok(DecisionProblem::for_task(task.description.clone()))
    }
}

#[derive(Debug, Default, Clone)]
pub struct SimplePlanDecoder;

#[async_trait]
impl PlanDecoder for SimplePlanDecoder {
    type Plan = Plan;

    async fn decode(
        &self,
        output: SolverOutput,
        metadata: ExecutionMetadata,
    ) -> Result<Self::Plan> {
        let steps = output
            .steps
            .into_iter()
            .map(|step| PlanStep {
                id: step.id,
                title: step.title,
                tool_name: step.tool_hint.unwrap_or_else(|| "workflow".into()),
                action_id: step.action_id,
                rationale: step.rationale,
                risk_level: if step.risk >= 0.85 {
                    "critical"
                } else if step.risk >= 0.55 {
                    "high"
                } else if step.risk >= 0.3 {
                    "medium"
                } else {
                    "low"
                }
                .into(),
            })
            .enumerate()
            .map(|(idx, mut step)| {
                if step.id.is_empty() {
                    step.id = format!("plan-step-{idx}");
                }
                step
            })
            .collect();

        Ok(Plan {
            id: format!("plan-{}", output.backend),
            backend: output.backend,
            backend_kind: output.backend_kind,
            steps,
            score: PlanScore {
                utility: output.score.utility,
                confidence: output.score.confidence,
                cost_estimate: output.score.cost_estimate,
                risk: output.score.risk,
            },
            rationale: PlannerRationale::new(output.rationale),
            metadata,
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct HybridPlanner {
    encoder: DefaultProblemEncoder,
    decoder: SimplePlanDecoder,
}

impl HybridPlanner {
    pub async fn plan(&self, request: PlannerRequest) -> Result<PlannerResponse> {
        self.plan_inner(request).await
    }

    async fn plan_inner(&self, request: PlannerRequest) -> Result<PlannerResponse> {
        let started = Instant::now();
        let problem = match request.problem {
            Some(problem) => problem,
            None => self.encoder.encode(&request.task).await?,
        };
        let primary = select_primary_backend(
            request.mode,
            &request.selection_policy,
            &request.backends,
            &problem,
        )?;
        let context = SolverContext::from_task(&request.task);
        let primary_started = Instant::now();
        let primary_output = primary.solve(problem.clone(), context.clone()).await?;
        let primary_latency_ms = primary_started.elapsed().as_millis() as u64;
        let primary_telemetry = primary_output.telemetry.clone();
        let primary_for_comparison = primary_output.clone();
        let mut plan = self
            .decoder
            .decode(primary_output, problem.metadata.clone())
            .await?;
        if !request.retrieved_skills.is_empty() {
            plan.rationale.selected_backend_reason = format!(
                "Selected {} after retrieving {} reusable procedures",
                primary.name(),
                request.retrieved_skills.len()
            );
        } else {
            plan.rationale.selected_backend_reason = format!(
                "Selected {} for planner mode {:?}",
                primary.name(),
                request.mode
            );
        }

        let mut backend_telemetry = vec![primary_telemetry];
        let mut shadow_comparison = None;
        if request.mode == PlannerMode::ShadowCompare && request.selection_policy.allow_shadow {
            if let Some(shadow) =
                select_shadow_backend(primary.kind(), request.shadow_backend, &request.backends)
            {
                let shadow_started = Instant::now();
                let shadow_output = shadow.solve(problem, context).await?;
                let shadow_plan = self
                    .decoder
                    .decode(shadow_output.clone(), ExecutionMetadata::default())
                    .await?;
                backend_telemetry.push(shadow_output.telemetry.clone());
                shadow_comparison = Some(ShadowComparison {
                    primary_backend: plan.backend.clone(),
                    primary_backend_kind: plan.backend_kind,
                    shadow_backend: shadow_plan.backend.clone(),
                    shadow_backend_kind: shadow_plan.backend_kind,
                    primary_score: plan.score.clone(),
                    shadow_score: shadow_plan.score,
                    latency_ms: shadow_started.elapsed().as_millis() as u64,
                    primary_latency_ms,
                    optimization: OptimizationComparison::between(
                        &primary_for_comparison,
                        &shadow_output,
                    ),
                });
            }
        }

        let rejected_backends = request
            .backends
            .iter()
            .filter(|backend| backend.name() != primary.name())
            .map(|backend| {
                format!(
                    "{} was not selected for mode {:?}",
                    backend.name(),
                    request.mode
                )
            })
            .collect::<Vec<_>>();

        let mut rationale = PlannerRationale::new("Planner encoded the task into backend-neutral IR, selected a solver, decoded a scored plan, and preserved execution metadata");
        rationale.selected_backend_reason = format!(
            "{} matched mode {:?} and policy {:?}",
            primary.name(),
            request.mode,
            request.selection_policy.preferred_kind
        );
        rationale.rejected_backend_reasons = rejected_backends.clone();

        Ok(PlannerResponse {
            telemetry: PlannerTelemetry {
                selected_backend: plan.backend.clone(),
                selected_backend_kind: plan.backend_kind,
                rejected_backends,
                plan_score: plan.score.clone(),
                latency_ms: started.elapsed().as_millis() as u64,
                shadow_comparison,
                backend_telemetry,
            },
            plans: vec![plan],
            rationale,
        })
    }
}

#[async_trait]
impl quantumclaw_core::Planner for HybridPlanner {
    type Request = PlannerRequest;
    type Response = PlannerResponse;

    async fn plan(&self, request: Self::Request) -> Result<Self::Response> {
        self.plan_inner(request).await
    }
}

fn select_primary_backend(
    mode: PlannerMode,
    policy: &BackendSelectionPolicy,
    backends: &[Arc<dyn SolverBackend>],
    problem: &DecisionProblem,
) -> Result<Arc<dyn SolverBackend>> {
    if backends.is_empty() {
        return Err("planner requires at least one solver backend".into());
    }

    if let Some(name) = policy.preferred_backend.as_deref() {
        return backends
            .iter()
            .find(|backend| backend.name() == name)
            .cloned()
            .ok_or_else(|| {
                let available = backends
                    .iter()
                    .map(|backend| backend.name())
                    .collect::<Vec<_>>()
                    .join(", ");
                QuantumClawError::new(format!(
                    "requested solver backend '{name}' is not available; available backends: {available}"
                ))
            });
    }

    let preferred = match mode {
        PlannerMode::ClassicalOnly | PlannerMode::Reactive => Some(SolverKind::Classical),
        PlannerMode::QuantumInspiredPreferred | PlannerMode::Deliberative => {
            Some(SolverKind::QuantumInspired)
        }
        PlannerMode::ShadowCompare => policy.preferred_kind.or(Some(SolverKind::Classical)),
        PlannerMode::Hybrid | PlannerMode::Auto => {
            if problem.candidate_actions.len() >= 4 {
                Some(SolverKind::QuantumInspired)
            } else {
                Some(SolverKind::Classical)
            }
        }
    }
    .or(policy.preferred_kind);

    if let Some(kind) = preferred {
        if let Some(backend) = backends.iter().find(|backend| backend.kind() == kind) {
            return Ok(backend.clone());
        }
    }

    Ok(backends[0].clone())
}

fn select_shadow_backend(
    primary_kind: SolverKind,
    explicit_shadow: Option<Arc<dyn SolverBackend>>,
    backends: &[Arc<dyn SolverBackend>],
) -> Option<Arc<dyn SolverBackend>> {
    explicit_shadow.or_else(|| {
        backends
            .iter()
            .find(|backend| backend.kind() != primary_kind)
            .cloned()
    })
}

//! Quantum brains: domain-specific optimization intelligence.
//!
//! A brain owns the knowledge of one problem domain — what a valid input looks
//! like, how to break a large instance into solvable pieces, how to turn those
//! pieces into binary optimization models, and how to judge the answer in the
//! domain's own units. It does **not** own solvers. Everything a brain wants
//! solved goes through [`quantumclaw_core::SolverBackend`], so a brain never
//! knows or cares whether the answer came from a classical heuristic,
//! simulated annealing, a hybrid solver, or a QPU.
//!
//! ```text
//! agent task -> BrainRegistry -> domain brain -> OptimizationProblem
//!                                                     |
//!                                              SolverBackend
//!                                                     |
//!                                     domain decoding + KPI evaluation
//! ```

use async_trait::async_trait;
use quantumclaw_core::{AgentTask, QuantumClawError, Result, SolverRegistry};
use quantumclaw_ir::optimization::OptimizationProblem;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Operations every brain exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrainOperation {
    Validate,
    Plan,
    Formulate,
    Decompose,
    Solve,
    Evaluate,
    Explain,
}

/// What a brain can do, advertised to agents and tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainCapabilities {
    pub id: String,
    pub domain: String,
    pub problem_classes: Vec<String>,
    pub supports_decomposition: bool,
    /// Largest instance the brain will accept, when it declares a limit.
    pub max_problem_size: Option<usize>,
}

impl BrainCapabilities {
    pub fn new(id: impl Into<String>, domain: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            domain: domain.into(),
            problem_classes: Vec::new(),
            supports_decomposition: false,
            max_problem_size: None,
        }
    }

    pub fn with_problem_class(mut self, class: impl Into<String>) -> Self {
        self.problem_classes.push(class.into());
        self
    }

    pub fn with_decomposition(mut self, supports: bool) -> Self {
        self.supports_decomposition = supports;
        self
    }
}

/// How well a brain matches a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainMatch {
    /// `0.0` means "not my domain"; higher means a better fit.
    pub score: f64,
    pub reason: String,
}

impl BrainMatch {
    pub fn none(reason: impl Into<String>) -> Self {
        Self {
            score: 0.0,
            reason: reason.into(),
        }
    }

    pub fn new(score: f64, reason: impl Into<String>) -> Self {
        Self {
            score: score.clamp(0.0, 1.0),
            reason: reason.into(),
        }
    }

    /// Scores a task by how many of a domain's terms it mentions.
    pub fn from_keywords(description: &str, keywords: &[String]) -> Self {
        if keywords.is_empty() {
            return Self::none("the brain declares no domain vocabulary");
        }
        let haystack = description.to_lowercase();
        let matched: Vec<&String> = keywords
            .iter()
            .filter(|keyword| haystack.contains(&keyword.to_lowercase()))
            .collect();
        if matched.is_empty() {
            return Self::none("the task mentions none of the domain's terms");
        }
        let score = (matched.len() as f64 / keywords.len() as f64).clamp(0.1, 1.0);
        Self::new(
            score,
            format!(
                "the task mentions {}",
                matched
                    .iter()
                    .map(|keyword| keyword.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    }
}

/// Severity of a validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: IssueSeverity,
    pub subject: String,
    pub message: String,
}

/// Result of validating a domain input.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn error(&mut self, subject: impl Into<String>, message: impl Into<String>) {
        self.issues.push(ValidationIssue {
            severity: IssueSeverity::Error,
            subject: subject.into(),
            message: message.into(),
        });
    }

    pub fn warn(&mut self, subject: impl Into<String>, message: impl Into<String>) {
        self.issues.push(ValidationIssue {
            severity: IssueSeverity::Warning,
            subject: subject.into(),
            message: message.into(),
        });
    }

    pub fn errors(&self) -> impl Iterator<Item = &ValidationIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity == IssueSeverity::Error)
    }

    /// Marks the report valid when it carries no errors.
    pub fn finish(mut self) -> Self {
        let has_errors = self
            .issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Error);
        self.valid = !has_errors;
        self
    }

    pub fn into_result(self) -> Result<Self> {
        let report = self.finish();
        if report.valid {
            return Ok(report);
        }
        let reasons = report
            .errors()
            .map(|issue| format!("{}: {}", issue.subject, issue.message))
            .collect::<Vec<_>>()
            .join("; ");
        Err(QuantumClawError::new(format!(
            "the input is not valid for this brain: {reasons}"
        )))
    }
}

/// A stage in the brain's approach to a problem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainStage {
    pub id: String,
    pub description: String,
    /// Which solver class the brain expects to use here, when it knows.
    pub solver_hint: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BrainPlan {
    pub stages: Vec<BrainStage>,
}

impl BrainPlan {
    pub fn stage(
        mut self,
        id: impl Into<String>,
        description: impl Into<String>,
        solver_hint: Option<String>,
    ) -> Self {
        self.stages.push(BrainStage {
            id: id.into(),
            description: description.into(),
            solver_hint,
        });
        self
    }
}

/// A binary optimization model the brain wants solved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Formulation {
    pub id: String,
    /// Domain-declared class, for example `vehicle-assignment`.
    pub class: String,
    pub problem: OptimizationProblem,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// How a large instance was split up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decomposition {
    pub strategy: String,
    pub rationale: String,
    pub subproblems: Vec<SubproblemSummary>,
}

impl Decomposition {
    pub fn single_block(strategy: impl Into<String>) -> Self {
        Self {
            strategy: strategy.into(),
            rationale: "the instance is small enough to solve in one piece".into(),
            subproblems: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubproblemSummary {
    pub id: String,
    pub class: String,
    pub size: usize,
    pub members: Vec<String>,
}

/// Domain metrics for a solved instance.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KpiReport {
    pub metrics: BTreeMap<String, f64>,
    pub notes: Vec<String>,
}

impl KpiReport {
    pub fn set(&mut self, name: impl Into<String>, value: f64) {
        self.metrics.insert(name.into(), value);
    }

    pub fn get(&self, name: &str) -> Option<f64> {
        self.metrics.get(name).copied()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Explanation {
    pub summary: String,
    pub details: Vec<String>,
}

impl Explanation {
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            details: Vec::new(),
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.details.push(detail.into());
        self
    }
}

/// Everything a brain needs to reach solvers.
#[derive(Clone, Default)]
pub struct BrainSolveContext {
    /// Backends the brain may use. An empty registry means the brain must fall
    /// back to its own classical methods.
    pub registry: Arc<SolverRegistry>,
    /// Backend the caller insists on, by name.
    pub requested_backend: Option<String>,
    /// Backends to run alongside the primary for comparison.
    pub shadow_backends: Vec<String>,
    /// The agent task that led here, when there was one.
    pub task: Option<AgentTask>,
}

impl BrainSolveContext {
    pub fn with_registry(mut self, registry: Arc<SolverRegistry>) -> Self {
        self.registry = registry;
        self
    }

    pub fn with_requested_backend(mut self, backend: impl Into<String>) -> Self {
        self.requested_backend = Some(backend.into());
        self
    }

    pub fn with_shadow_backend(mut self, backend: impl Into<String>) -> Self {
        self.shadow_backends.push(backend.into());
        self
    }
}

impl std::fmt::Debug for BrainSolveContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrainSolveContext")
            .field("backends", &self.registry.names())
            .field("requested_backend", &self.requested_backend)
            .field("shadow_backends", &self.shadow_backends)
            .finish()
    }
}

/// A domain brain.
///
/// Implementors own domain knowledge only. Solver selection, QUBO compilation
/// mechanics, and provider integration all live outside.
#[async_trait]
pub trait QuantumBrain: Send + Sync {
    type Input: Send + Sync + 'static;
    type Output: Send + Sync + 'static;

    fn id(&self) -> &str;
    fn capabilities(&self) -> BrainCapabilities;

    /// How well this brain fits an agent task.
    fn can_handle(&self, task: &AgentTask) -> BrainMatch;

    async fn validate(&self, input: &Self::Input) -> Result<ValidationReport>;
    async fn plan(&self, input: &Self::Input) -> Result<BrainPlan>;
    async fn formulate(&self, input: &Self::Input) -> Result<Vec<Formulation>>;
    async fn decompose(&self, input: &Self::Input) -> Result<Decomposition>;
    async fn solve(&self, input: Self::Input, context: BrainSolveContext) -> Result<Self::Output>;
    async fn evaluate(&self, output: &Self::Output) -> Result<KpiReport>;
    async fn explain(&self, output: &Self::Output) -> Result<Explanation>;
}

/// Object-safe view of a brain, used by registries, tools, and agents.
#[async_trait]
pub trait ErasedBrain: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> BrainCapabilities;
    fn can_handle(&self, task: &AgentTask) -> BrainMatch;

    /// Runs one operation with JSON in and JSON out.
    async fn run(
        &self,
        operation: BrainOperation,
        input: Value,
        context: BrainSolveContext,
    ) -> Result<Value>;
}

/// Adapts any [`QuantumBrain`] with serializable types into an [`ErasedBrain`].
pub struct JsonBrain<B> {
    brain: Arc<B>,
}

impl<B> JsonBrain<B> {
    pub fn new(brain: Arc<B>) -> Self {
        Self { brain }
    }

    pub fn inner(&self) -> &Arc<B> {
        &self.brain
    }
}

impl<B> std::fmt::Debug for JsonBrain<B>
where
    B: QuantumBrain,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsonBrain")
            .field("id", &self.brain.id())
            .finish()
    }
}

#[async_trait]
impl<B> ErasedBrain for JsonBrain<B>
where
    B: QuantumBrain,
    B::Input: DeserializeOwned,
    B::Output: Serialize + DeserializeOwned,
{
    fn id(&self) -> &str {
        self.brain.id()
    }

    fn capabilities(&self) -> BrainCapabilities {
        self.brain.capabilities()
    }

    fn can_handle(&self, task: &AgentTask) -> BrainMatch {
        self.brain.can_handle(task)
    }

    async fn run(
        &self,
        operation: BrainOperation,
        input: Value,
        context: BrainSolveContext,
    ) -> Result<Value> {
        let id = self.brain.id().to_string();
        match operation {
            BrainOperation::Evaluate => {
                let output = self.decode_output(input)?;
                encode(&id, self.brain.evaluate(&output).await?)
            }
            BrainOperation::Explain => {
                let output = self.decode_output(input)?;
                encode(&id, self.brain.explain(&output).await?)
            }
            BrainOperation::Validate => {
                let decoded = self.decode_input(input)?;
                encode(&id, self.brain.validate(&decoded).await?.finish())
            }
            BrainOperation::Plan => {
                let decoded = self.decode_input(input)?;
                encode(&id, self.brain.plan(&decoded).await?)
            }
            BrainOperation::Formulate => {
                let decoded = self.decode_input(input)?;
                encode(&id, self.brain.formulate(&decoded).await?)
            }
            BrainOperation::Decompose => {
                let decoded = self.decode_input(input)?;
                encode(&id, self.brain.decompose(&decoded).await?)
            }
            BrainOperation::Solve => {
                let decoded = self.decode_input(input)?;
                encode(&id, self.brain.solve(decoded, context).await?)
            }
        }
    }
}

impl<B> JsonBrain<B>
where
    B: QuantumBrain,
    B::Input: DeserializeOwned,
    B::Output: DeserializeOwned,
{
    fn decode_input(&self, input: Value) -> Result<B::Input> {
        serde_json::from_value(input).map_err(|error| {
            QuantumClawError::new(format!(
                "brain '{}' could not read the request payload: {error}",
                self.brain.id()
            ))
        })
    }

    fn decode_output(&self, input: Value) -> Result<B::Output> {
        serde_json::from_value(input).map_err(|error| {
            QuantumClawError::new(format!(
                "brain '{}' could not read the result payload: {error}",
                self.brain.id()
            ))
        })
    }
}

fn encode<T: Serialize>(id: &str, value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|error| {
        QuantumClawError::new(format!(
            "brain '{id}' produced an unserializable result: {error}"
        ))
    })
}

/// A brain and the score that selected it.
#[derive(Clone)]
pub struct BrainSelection {
    pub brain: Arc<dyn ErasedBrain>,
    pub match_result: BrainMatch,
}

/// Registry of domain brains, used to route agent tasks.
#[derive(Default, Clone)]
pub struct BrainRegistry {
    brains: Vec<Arc<dyn ErasedBrain>>,
    /// Minimum match score a brain needs before it is selected.
    pub selection_threshold: f64,
}

impl BrainRegistry {
    pub fn new() -> Self {
        Self {
            brains: Vec::new(),
            selection_threshold: f64::EPSILON,
        }
    }

    pub fn register(&mut self, brain: Arc<dyn ErasedBrain>) -> &mut Self {
        self.brains.push(brain);
        self
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn ErasedBrain>> {
        self.brains.iter().find(|brain| brain.id() == id).cloned()
    }

    pub fn require(&self, id: &str) -> Result<Arc<dyn ErasedBrain>> {
        self.get(id).ok_or_else(|| {
            QuantumClawError::new(format!(
                "unknown brain '{id}'; available brains: {}",
                self.ids().join(", ")
            ))
        })
    }

    pub fn ids(&self) -> Vec<String> {
        self.brains.iter().map(|brain| brain.id().into()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.brains.is_empty()
    }

    /// Picks the best-matching brain for a task, if any clears the threshold.
    pub fn select(&self, task: &AgentTask) -> Option<BrainSelection> {
        self.brains
            .iter()
            .map(|brain| BrainSelection {
                brain: brain.clone(),
                match_result: brain.can_handle(task),
            })
            .filter(|selection| selection.match_result.score >= self.selection_threshold)
            .max_by(|left, right| left.match_result.score.total_cmp(&right.match_result.score))
    }
}

impl std::fmt::Debug for BrainRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrainRegistry")
            .field("brains", &self.ids())
            .finish()
    }
}

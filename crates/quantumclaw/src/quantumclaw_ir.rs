use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionProblem {
    pub id: String,
    pub goals: Vec<Goal>,
    pub subtasks: Vec<Subtask>,
    pub candidate_actions: Vec<CandidateAction>,
    pub dependencies: Vec<Dependency>,
    pub constraints: Vec<Constraint>,
    pub cost_model: CostModel,
    pub risk_model: RiskModel,
    pub deadline: Option<Deadline>,
    pub resource_budget: ResourceBudget,
    pub confidence: ConfidenceEstimate,
    pub rollback: RollbackStrategy,
    pub metadata: ExecutionMetadata,
}

impl DecisionProblem {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            goals: Vec::new(),
            subtasks: Vec::new(),
            candidate_actions: Vec::new(),
            dependencies: Vec::new(),
            constraints: Vec::new(),
            cost_model: CostModel::default(),
            risk_model: RiskModel::default(),
            deadline: None,
            resource_budget: ResourceBudget::default(),
            confidence: ConfidenceEstimate::default(),
            rollback: RollbackStrategy::default(),
            metadata: ExecutionMetadata::default(),
        }
    }

    pub fn for_task(task: impl Into<String>) -> Self {
        let task = task.into();
        let mut problem = Self::new("decision-problem");
        problem
            .goals
            .push(Goal::new("goal", task.clone()).with_priority(1.0));
        problem.subtasks = vec![
            Subtask::new(
                "understand-context",
                "Understand the requested work and current code shape",
            ),
            Subtask::new(
                "design-safe-plan",
                "Design a small-step execution plan with validation gates",
            ),
            Subtask::new(
                "simulate-execution",
                "Resolve tools and simulate the execution path",
            ),
            Subtask::new(
                "capture-learning",
                "Capture reusable procedure after a successful execution",
            ),
        ];
        problem.candidate_actions = vec![
            CandidateAction::new(
                "inspect",
                "Inspect target module",
                "Read relevant files and interfaces",
            )
            .with_tool_hint("filesystem")
            .with_utility(0.78),
            CandidateAction::new(
                "test-first",
                "Add characterization tests",
                "Prove intended behavior before edits",
            )
            .with_tool_hint("shell")
            .with_utility(0.9),
            CandidateAction::new(
                "small-edit",
                "Apply focused code edit",
                "Change the smallest safe surface area",
            )
            .with_tool_hint("code_edit")
            .with_utility(0.86),
            CandidateAction::new(
                "validate",
                "Run validation suite",
                "Run formatter and tests before completion",
            )
            .with_tool_hint("shell")
            .with_utility(0.93),
        ];
        problem.dependencies = vec![
            Dependency::new(
                "inspect",
                "test-first",
                "Tests depend on knowing existing behavior",
            ),
            Dependency::new(
                "test-first",
                "small-edit",
                "Edits follow failing or characterization tests",
            ),
            Dependency::new(
                "small-edit",
                "validate",
                "Validation follows implementation",
            ),
        ];
        problem.constraints = vec![
            Constraint::hard(
                "validation-required",
                "No execution plan is complete without deterministic validation",
            ),
            Constraint::hard(
                "reversible-edits",
                "Prefer edits that can be rolled back cleanly",
            ),
            Constraint::soft(
                "low-blast-radius",
                "Minimize touched files and external side effects",
                0.8,
            ),
        ];
        problem.resource_budget = ResourceBudget {
            max_tool_calls: Some(12),
            max_latency_ms: Some(30_000),
            max_cost_micros: Some(50_000),
        };
        problem.metadata.tags = vec!["general-agent-runtime".into(), "safe-refactor".into()];
        problem.metadata.data.insert("task".into(), task);
        problem
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub description: String,
    pub priority: UtilityScore,
    pub success_criteria: Vec<String>,
}

impl Goal {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            priority: UtilityScore(1.0),
            success_criteria: Vec::new(),
        }
    }

    pub fn with_priority(mut self, value: f64) -> Self {
        self.priority = UtilityScore(value);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subtask {
    pub id: String,
    pub description: String,
    pub required: bool,
    pub estimated_cost: CostModel,
    pub risk: RiskModel,
}

impl Subtask {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            required: true,
            estimated_cost: CostModel::default(),
            risk: RiskModel::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateAction {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tool_hint: Option<String>,
    pub preconditions: Preconditions,
    pub postconditions: Postconditions,
    pub cost: CostModel,
    pub risk: RiskModel,
    pub utility: UtilityScore,
    pub rollback: RollbackStrategy,
}

impl CandidateAction {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            tool_hint: None,
            preconditions: Preconditions::default(),
            postconditions: Postconditions::default(),
            cost: CostModel::default(),
            risk: RiskModel::default(),
            utility: UtilityScore(0.5),
            rollback: RollbackStrategy::default(),
        }
    }

    pub fn with_tool_hint(mut self, tool: impl Into<String>) -> Self {
        self.tool_hint = Some(tool.into());
        self
    }

    pub fn with_utility(mut self, value: f64) -> Self {
        self.utility = UtilityScore(value);
        self
    }

    pub fn with_risk(mut self, score: f64) -> Self {
        self.risk.score = score;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub before: String,
    pub after: String,
    pub reason: String,
}

impl Dependency {
    pub fn new(
        before: impl Into<String>,
        after: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            before: before.into(),
            after: after.into(),
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Preconditions {
    pub checks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Postconditions {
    pub checks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    pub id: String,
    pub description: String,
    pub kind: ConstraintKind,
    pub weight: f64,
}

impl Constraint {
    pub fn hard(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            kind: ConstraintKind::Hard,
            weight: 1.0,
        }
    }

    pub fn soft(id: impl Into<String>, description: impl Into<String>, weight: f64) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            kind: ConstraintKind::Soft,
            weight,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstraintKind {
    Hard,
    Soft,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CostModel {
    pub estimated_latency_ms: u64,
    pub estimated_tokens: u64,
    pub estimated_money_micros: u64,
}

impl Default for CostModel {
    fn default() -> Self {
        Self {
            estimated_latency_ms: 100,
            estimated_tokens: 0,
            estimated_money_micros: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskModel {
    pub score: f64,
    pub labels: Vec<String>,
}

impl Default for RiskModel {
    fn default() -> Self {
        Self {
            score: 0.1,
            labels: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct UtilityScore(pub f64);

impl Default for UtilityScore {
    fn default() -> Self {
        Self(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deadline {
    pub unix_epoch_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBudget {
    pub max_tool_calls: Option<u32>,
    pub max_latency_ms: Option<u64>,
    pub max_cost_micros: Option<u64>,
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            max_tool_calls: Some(16),
            max_latency_ms: Some(60_000),
            max_cost_micros: Some(100_000),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ConfidenceEstimate(pub f64);

impl Default for ConfidenceEstimate {
    fn default() -> Self {
        Self(0.5)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackStrategy {
    pub kind: RollbackKind,
    pub description: String,
}

impl Default for RollbackStrategy {
    fn default() -> Self {
        Self {
            kind: RollbackKind::Checkpoint,
            description: "Preserve state before execution and revert on validation failure".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RollbackKind {
    None,
    Checkpoint,
    CompensatingAction,
    ManualReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionMetadata {
    pub trace_id: String,
    pub tags: Vec<String>,
    pub data: BTreeMap<String, String>,
}

impl Default for ExecutionMetadata {
    fn default() -> Self {
        Self {
            trace_id: "trace-local".into(),
            tags: Vec::new(),
            data: BTreeMap::new(),
        }
    }
}

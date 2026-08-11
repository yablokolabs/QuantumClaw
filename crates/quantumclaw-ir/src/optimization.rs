//! Backend-neutral combinatorial optimization model.
//!
//! These types describe binary optimization problems and their solutions
//! without referencing any solver, provider, or application domain. A route
//! optimizer, a scheduler, and a portfolio allocator all express their
//! combinatorial core with the same constructs, and every solver backend
//! consumes the same representation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Optimization direction of the objective function.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectiveSense {
    #[default]
    Minimize,
    Maximize,
}

/// A binary decision variable and the domain metadata needed to decode it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BinaryVariable {
    pub name: String,
    /// Domain-owned decoding hints, for example `delivery=d-1`, `vehicle=v-2`.
    pub metadata: BTreeMap<String, String>,
}

impl BinaryVariable {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// A single-variable term.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearTerm {
    pub variable: String,
    pub coefficient: f64,
}

impl LinearTerm {
    pub fn new(variable: impl Into<String>, coefficient: f64) -> Self {
        Self {
            variable: variable.into(),
            coefficient,
        }
    }
}

/// A two-variable interaction term.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuadraticTerm {
    pub first: String,
    pub second: String,
    pub coefficient: f64,
}

impl QuadraticTerm {
    pub fn new(first: impl Into<String>, second: impl Into<String>, coefficient: f64) -> Self {
        Self {
            first: first.into(),
            second: second.into(),
            coefficient,
        }
    }
}

/// Reusable constraint constructs shared by every optimization domain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConstraintExpression {
    /// Exactly one of the variables is selected.
    ExactlyOne { variables: Vec<String> },
    /// At most one of the variables is selected.
    AtMostOne { variables: Vec<String> },
    /// At least one of the variables is selected.
    AtLeastOne { variables: Vec<String> },
    /// Selecting `antecedent` requires selecting `consequent`.
    Implication {
        antecedent: String,
        consequent: String,
    },
    /// The two variables cannot both be selected.
    Conflict { first: String, second: String },
    /// Weighted sum of variables must equal `rhs`.
    LinearEquality { terms: Vec<LinearTerm>, rhs: f64 },
    /// Weighted sum of variables must not exceed `rhs`.
    LinearAtMost { terms: Vec<LinearTerm>, rhs: f64 },
}

/// A constraint plus the penalty weight used when it is compiled into a QUBO.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationConstraint {
    pub id: String,
    pub expression: ConstraintExpression,
    /// Explicit penalty weight. When absent the compiler derives one from the
    /// objective magnitude so that violating the constraint cannot pay off.
    pub penalty: Option<f64>,
    /// Hard constraints decide feasibility. Soft constraints are reported as
    /// violations without marking the solution infeasible.
    pub hard: bool,
}

impl OptimizationConstraint {
    pub fn new(id: impl Into<String>, expression: ConstraintExpression) -> Self {
        Self {
            id: id.into(),
            expression,
            penalty: None,
            hard: true,
        }
    }

    pub fn exactly_one<I, S>(id: impl Into<String>, variables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new(
            id,
            ConstraintExpression::ExactlyOne {
                variables: variables.into_iter().map(Into::into).collect(),
            },
        )
    }

    pub fn at_most_one<I, S>(id: impl Into<String>, variables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new(
            id,
            ConstraintExpression::AtMostOne {
                variables: variables.into_iter().map(Into::into).collect(),
            },
        )
    }

    pub fn at_least_one<I, S>(id: impl Into<String>, variables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new(
            id,
            ConstraintExpression::AtLeastOne {
                variables: variables.into_iter().map(Into::into).collect(),
            },
        )
    }

    pub fn implication(
        id: impl Into<String>,
        antecedent: impl Into<String>,
        consequent: impl Into<String>,
    ) -> Self {
        Self::new(
            id,
            ConstraintExpression::Implication {
                antecedent: antecedent.into(),
                consequent: consequent.into(),
            },
        )
    }

    pub fn conflict(
        id: impl Into<String>,
        first: impl Into<String>,
        second: impl Into<String>,
    ) -> Self {
        Self::new(
            id,
            ConstraintExpression::Conflict {
                first: first.into(),
                second: second.into(),
            },
        )
    }

    pub fn linear_at_most(id: impl Into<String>, terms: Vec<LinearTerm>, rhs: f64) -> Self {
        Self::new(id, ConstraintExpression::LinearAtMost { terms, rhs })
    }

    pub fn linear_equality(id: impl Into<String>, terms: Vec<LinearTerm>, rhs: f64) -> Self {
        Self::new(id, ConstraintExpression::LinearEquality { terms, rhs })
    }

    pub fn with_penalty(mut self, penalty: f64) -> Self {
        self.penalty = Some(penalty);
        self
    }

    pub fn soft(mut self) -> Self {
        self.hard = false;
        self
    }
}

/// A domain-neutral binary optimization problem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationProblem {
    pub id: String,
    pub sense: ObjectiveSense,
    pub variables: Vec<BinaryVariable>,
    pub linear: Vec<LinearTerm>,
    pub quadratic: Vec<QuadraticTerm>,
    pub offset: f64,
    pub constraints: Vec<OptimizationConstraint>,
    pub metadata: BTreeMap<String, String>,
}

impl OptimizationProblem {
    pub fn new(id: impl Into<String>, sense: ObjectiveSense) -> Self {
        Self {
            id: id.into(),
            sense,
            variables: Vec::new(),
            linear: Vec::new(),
            quadratic: Vec::new(),
            offset: 0.0,
            constraints: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    pub fn minimize(id: impl Into<String>) -> Self {
        Self::new(id, ObjectiveSense::Minimize)
    }

    pub fn maximize(id: impl Into<String>) -> Self {
        Self::new(id, ObjectiveSense::Maximize)
    }

    pub fn with_variable(mut self, variable: BinaryVariable) -> Self {
        self.variables.push(variable);
        self
    }

    /// Declare a variable and its objective coefficient in one step.
    pub fn with_term(mut self, name: impl Into<String>, coefficient: f64) -> Self {
        let name = name.into();
        if !self.variables.iter().any(|variable| variable.name == name) {
            self.variables.push(BinaryVariable::new(name.clone()));
        }
        self.linear.push(LinearTerm::new(name, coefficient));
        self
    }

    pub fn with_interaction(
        mut self,
        first: impl Into<String>,
        second: impl Into<String>,
        coefficient: f64,
    ) -> Self {
        self.quadratic
            .push(QuadraticTerm::new(first, second, coefficient));
        self
    }

    pub fn with_constraint(mut self, constraint: OptimizationConstraint) -> Self {
        self.constraints.push(constraint);
        self
    }

    pub fn with_offset(mut self, offset: f64) -> Self {
        self.offset = offset;
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    pub fn variable_names(&self) -> Vec<String> {
        self.variables
            .iter()
            .map(|variable| variable.name.clone())
            .collect()
    }

    pub fn variable(&self, name: &str) -> Option<&BinaryVariable> {
        self.variables.iter().find(|variable| variable.name == name)
    }

    /// Objective value of an assignment in the problem's own units.
    pub fn objective_value(&self, assignments: &BTreeMap<String, u8>) -> f64 {
        let value = |name: &str| f64::from(assignments.get(name).copied().unwrap_or(0));
        let linear: f64 = self
            .linear
            .iter()
            .map(|term| term.coefficient * value(&term.variable))
            .sum();
        let quadratic: f64 = self
            .quadratic
            .iter()
            .map(|term| term.coefficient * value(&term.first) * value(&term.second))
            .sum();
        linear + quadratic + self.offset
    }
}

/// A binary quadratic model in minimization form.
///
/// This is the representation handed to samplers. It is expressed with binary
/// (`0`/`1`) variables, which maps directly onto an Ocean `BinaryQuadraticModel`
/// with `Vartype.BINARY` and onto any other QUBO-capable solver.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BinaryQuadraticModel {
    pub variables: Vec<String>,
    pub linear: Vec<LinearTerm>,
    pub quadratic: Vec<QuadraticTerm>,
    pub offset: f64,
}

impl BinaryQuadraticModel {
    pub fn num_variables(&self) -> usize {
        self.variables.len()
    }

    pub fn num_interactions(&self) -> usize {
        self.quadratic.len()
    }

    /// Energy of an assignment under this minimization model.
    pub fn energy(&self, assignments: &BTreeMap<String, u8>) -> f64 {
        let value = |name: &str| f64::from(assignments.get(name).copied().unwrap_or(0));
        let linear: f64 = self
            .linear
            .iter()
            .map(|term| term.coefficient * value(&term.variable))
            .sum();
        let quadratic: f64 = self
            .quadratic
            .iter()
            .map(|term| term.coefficient * value(&term.first) * value(&term.second))
            .sum();
        linear + quadratic + self.offset
    }
}

/// A constraint that an assignment failed to satisfy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstraintViolation {
    pub constraint_id: String,
    pub description: String,
    /// How far the assignment is from satisfying the constraint.
    pub magnitude: f64,
    pub hard: bool,
}

/// A decoded, normalized solution to an [`OptimizationProblem`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationSolution {
    pub problem_id: String,
    /// Assignment of the problem's declared variables. Compiler-introduced
    /// slack variables are not reported here.
    pub assignments: BTreeMap<String, u8>,
    pub selected: Vec<String>,
    /// Objective value in the problem's original sense and units, excluding
    /// constraint penalties.
    pub objective_value: f64,
    /// Energy of the compiled minimization model, including penalties.
    pub energy: f64,
    pub sense: ObjectiveSense,
    pub feasible: bool,
    pub violations: Vec<ConstraintViolation>,
    pub metadata: BTreeMap<String, String>,
}

impl OptimizationSolution {
    pub fn hard_violations(&self) -> impl Iterator<Item = &ConstraintViolation> {
        self.violations.iter().filter(|violation| violation.hard)
    }

    /// Domain metadata of the selected variables, in selection order.
    pub fn selected_metadata<'a>(
        &'a self,
        problem: &'a OptimizationProblem,
    ) -> Vec<&'a BTreeMap<String, String>> {
        self.selected
            .iter()
            .filter_map(|name| problem.variable(name))
            .map(|variable| &variable.metadata)
            .collect()
    }
}

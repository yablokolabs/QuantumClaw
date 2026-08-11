use crate::error::{OptimizationError, Result};
use quantumclaw_ir::optimization::{
    BinaryQuadraticModel, ConstraintExpression, ConstraintViolation, LinearTerm, ObjectiveSense,
    OptimizationConstraint, OptimizationProblem, OptimizationSolution, QuadraticTerm,
};
use std::collections::BTreeMap;

/// Tolerance used when checking that inequality coefficients are integral.
const INTEGRALITY_TOLERANCE: f64 = 1e-9;

/// A slack variable introduced by the compiler to encode an inequality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackVariable {
    pub name: String,
    pub constraint_id: String,
    pub weight: i64,
}

/// The result of compiling an [`OptimizationProblem`] into a QUBO.
///
/// The model is always in minimization form because that is what samplers
/// expect. The original objective sense is retained so decoded solutions can
/// report objective values in the units the caller supplied.
#[derive(Debug, Clone)]
pub struct CompiledModel {
    problem: OptimizationProblem,
    bqm: BinaryQuadraticModel,
    slack_variables: Vec<SlackVariable>,
    penalties: BTreeMap<String, f64>,
    max_exhaustive_variables: usize,
}

/// Compiles optimization problems into binary quadratic models.
#[derive(Debug, Clone)]
pub struct QuboCompiler {
    /// Multiplier applied to the objective magnitude when a constraint does not
    /// carry an explicit penalty.
    pub penalty_factor: f64,
    /// Maximum number of slack bits allowed per inequality constraint.
    pub max_slack_bits: u32,
    /// Maximum number of variables an exhaustive search will consider.
    pub max_exhaustive_variables: usize,
}

impl Default for QuboCompiler {
    fn default() -> Self {
        Self {
            penalty_factor: 2.0,
            max_slack_bits: 16,
            max_exhaustive_variables: 20,
        }
    }
}

impl QuboCompiler {
    pub fn with_penalty_factor(mut self, penalty_factor: f64) -> Self {
        self.penalty_factor = penalty_factor;
        self
    }

    pub fn with_max_exhaustive_variables(mut self, max_exhaustive_variables: usize) -> Self {
        self.max_exhaustive_variables = max_exhaustive_variables;
        self
    }

    /// Compiles the problem into a minimization QUBO with penalty-encoded
    /// constraints.
    pub fn compile(&self, problem: &OptimizationProblem) -> Result<CompiledModel> {
        if problem.variables.is_empty() {
            return Err(OptimizationError::EmptyProblem {
                problem_id: problem.id.clone(),
            });
        }

        let declared: Vec<String> = problem.variable_names();
        let mut builder = QuboBuilder::new(&declared);
        let sign = match problem.sense {
            ObjectiveSense::Minimize => 1.0,
            ObjectiveSense::Maximize => -1.0,
        };

        for term in &problem.linear {
            builder.check_declared("objective", &term.variable)?;
            builder.add_linear(&term.variable, sign * term.coefficient);
        }
        for term in &problem.quadratic {
            builder.check_declared("objective", &term.first)?;
            builder.check_declared("objective", &term.second)?;
            builder.add_quadratic(&term.first, &term.second, sign * term.coefficient);
        }
        builder.add_offset(sign * problem.offset);

        let magnitude = builder.magnitude();
        let mut penalties = BTreeMap::new();
        for constraint in &problem.constraints {
            let penalty = self.penalty_for(constraint, magnitude)?;
            penalties.insert(constraint.id.clone(), penalty);
            self.apply_constraint(&mut builder, constraint, penalty)?;
        }

        let (bqm, slack_variables) = builder.finish();
        Ok(CompiledModel {
            problem: problem.clone(),
            bqm,
            slack_variables,
            penalties,
            max_exhaustive_variables: self.max_exhaustive_variables,
        })
    }

    fn penalty_for(&self, constraint: &OptimizationConstraint, magnitude: f64) -> Result<f64> {
        let penalty = constraint
            .penalty
            .unwrap_or(self.penalty_factor * magnitude);
        if !penalty.is_finite() || penalty <= 0.0 {
            return Err(OptimizationError::InvalidPenalty {
                constraint_id: constraint.id.clone(),
                reason: format!("penalty must be finite and positive, got {penalty}"),
            });
        }
        Ok(penalty)
    }

    fn apply_constraint(
        &self,
        builder: &mut QuboBuilder,
        constraint: &OptimizationConstraint,
        penalty: f64,
    ) -> Result<()> {
        match &constraint.expression {
            ConstraintExpression::ExactlyOne { variables } => {
                let terms = unit_terms(builder, &constraint.id, variables)?;
                builder.add_square(&terms, -1.0, penalty);
            }
            ConstraintExpression::AtMostOne { variables } => {
                let terms = unit_terms(builder, &constraint.id, variables)?;
                for (index, (first, _)) in terms.iter().enumerate() {
                    for (second, _) in terms.iter().skip(index + 1) {
                        builder.add_quadratic(first, second, penalty);
                    }
                }
            }
            ConstraintExpression::AtLeastOne { variables } => {
                let terms: Vec<LinearTerm> = variables
                    .iter()
                    .map(|variable| LinearTerm::new(variable, -1.0))
                    .collect();
                self.apply_at_most(builder, constraint, &terms, -1.0, penalty)?;
            }
            ConstraintExpression::Implication {
                antecedent,
                consequent,
            } => {
                builder.check_declared(&constraint.id, antecedent)?;
                builder.check_declared(&constraint.id, consequent)?;
                // penalty * antecedent * (1 - consequent)
                builder.add_linear(antecedent, penalty);
                builder.add_quadratic(antecedent, consequent, -penalty);
            }
            ConstraintExpression::Conflict { first, second } => {
                builder.check_declared(&constraint.id, first)?;
                builder.check_declared(&constraint.id, second)?;
                builder.add_quadratic(first, second, penalty);
            }
            ConstraintExpression::LinearEquality { terms, rhs } => {
                let terms = weighted_terms(builder, &constraint.id, terms)?;
                builder.add_square(&terms, -rhs, penalty);
            }
            ConstraintExpression::LinearAtMost { terms, rhs } => {
                self.apply_at_most(builder, constraint, terms, *rhs, penalty)?;
            }
        }
        Ok(())
    }

    /// Encodes `sum(terms) <= rhs` as `(sum(terms) + slack - rhs)^2`.
    fn apply_at_most(
        &self,
        builder: &mut QuboBuilder,
        constraint: &OptimizationConstraint,
        terms: &[LinearTerm],
        rhs: f64,
        penalty: f64,
    ) -> Result<()> {
        let mut weighted = weighted_terms(builder, &constraint.id, terms)?;
        require_integral(&constraint.id, &weighted, rhs)?;

        let minimum: f64 = weighted
            .iter()
            .map(|(_, coefficient)| coefficient.min(0.0))
            .sum();
        let bound = rhs - minimum;
        if bound < -INTEGRALITY_TOLERANCE {
            return Err(OptimizationError::UnsupportedConstraint {
                constraint_id: constraint.id.clone(),
                reason: format!(
                    "no assignment can satisfy it: the smallest reachable sum is {minimum} but the limit is {rhs}"
                ),
            });
        }

        for (index, weight) in slack_weights(bound.round().max(0.0) as u64)
            .into_iter()
            .enumerate()
        {
            if index as u32 >= self.max_slack_bits {
                return Err(OptimizationError::SlackOverflow {
                    constraint_id: constraint.id.clone(),
                    required_bits: index as u32 + 1,
                    max_bits: self.max_slack_bits,
                });
            }
            let name = format!("__slack__{}__{index}", constraint.id);
            builder.declare_slack(SlackVariable {
                name: name.clone(),
                constraint_id: constraint.id.clone(),
                weight: weight as i64,
            });
            weighted.push((name, weight as f64));
        }

        builder.add_square(&weighted, -rhs, penalty);
        Ok(())
    }
}

impl CompiledModel {
    pub fn problem(&self) -> &OptimizationProblem {
        &self.problem
    }

    pub fn bqm(&self) -> &BinaryQuadraticModel {
        &self.bqm
    }

    pub fn slack_variables(&self) -> &[SlackVariable] {
        &self.slack_variables
    }

    /// Penalty weight the compiler applied to a constraint.
    pub fn applied_penalty(&self, constraint_id: &str) -> Option<f64> {
        self.penalties.get(constraint_id).copied()
    }

    /// Turns a raw sample into a normalized solution.
    ///
    /// Variables missing from the sample are treated as unselected. Constraint
    /// satisfaction is evaluated against the original constraint semantics, not
    /// against the penalty terms, so a decoded solution reports what a domain
    /// owner would recognize as a violation.
    pub fn decode(&self, sample: &BTreeMap<String, u8>) -> OptimizationSolution {
        let assignments: BTreeMap<String, u8> = self
            .problem
            .variables
            .iter()
            .map(|variable| {
                let value = sample.get(&variable.name).copied().unwrap_or(0);
                (variable.name.clone(), u8::from(value != 0))
            })
            .collect();
        let selected: Vec<String> = self
            .problem
            .variables
            .iter()
            .filter(|variable| assignments.get(&variable.name).copied().unwrap_or(0) == 1)
            .map(|variable| variable.name.clone())
            .collect();

        let violations = self.violations(&assignments);
        let feasible = !violations.iter().any(|violation| violation.hard);

        let mut metadata = BTreeMap::new();
        metadata.insert(
            "model_variables".into(),
            self.bqm.num_variables().to_string(),
        );
        metadata.insert(
            "slack_variables".into(),
            self.slack_variables.len().to_string(),
        );

        OptimizationSolution {
            problem_id: self.problem.id.clone(),
            objective_value: self.problem.objective_value(&assignments),
            energy: self.bqm.energy(sample),
            sense: self.problem.sense,
            assignments,
            selected,
            feasible,
            violations,
            metadata,
        }
    }

    /// Constraints the assignment fails to satisfy, in declaration order.
    pub fn violations(&self, assignments: &BTreeMap<String, u8>) -> Vec<ConstraintViolation> {
        let value = |name: &str| f64::from(assignments.get(name).copied().unwrap_or(0));
        let mut violations = Vec::new();

        for constraint in &self.problem.constraints {
            let (magnitude, description) = match &constraint.expression {
                ConstraintExpression::ExactlyOne { variables } => {
                    let count: f64 = variables.iter().map(|name| value(name)).sum();
                    (
                        (count - 1.0).abs(),
                        format!("expected exactly one selection, found {count}"),
                    )
                }
                ConstraintExpression::AtMostOne { variables } => {
                    let count: f64 = variables.iter().map(|name| value(name)).sum();
                    (
                        (count - 1.0).max(0.0),
                        format!("expected at most one selection, found {count}"),
                    )
                }
                ConstraintExpression::AtLeastOne { variables } => {
                    let count: f64 = variables.iter().map(|name| value(name)).sum();
                    (
                        (1.0 - count).max(0.0),
                        format!("expected at least one selection, found {count}"),
                    )
                }
                ConstraintExpression::Implication {
                    antecedent,
                    consequent,
                } => {
                    let violated = value(antecedent) > 0.0 && value(consequent) == 0.0;
                    (
                        f64::from(violated),
                        format!("'{antecedent}' requires '{consequent}'"),
                    )
                }
                ConstraintExpression::Conflict { first, second } => {
                    let violated = value(first) > 0.0 && value(second) > 0.0;
                    (
                        f64::from(violated),
                        format!("'{first}' and '{second}' cannot both be selected"),
                    )
                }
                ConstraintExpression::LinearEquality { terms, rhs } => {
                    let total: f64 = terms
                        .iter()
                        .map(|term| term.coefficient * value(&term.variable))
                        .sum();
                    (
                        (total - rhs).abs(),
                        format!("expected a total of {rhs}, found {total}"),
                    )
                }
                ConstraintExpression::LinearAtMost { terms, rhs } => {
                    let total: f64 = terms
                        .iter()
                        .map(|term| term.coefficient * value(&term.variable))
                        .sum();
                    (
                        (total - rhs).max(0.0),
                        format!("total {total} exceeds the limit of {rhs}"),
                    )
                }
            };

            if magnitude > INTEGRALITY_TOLERANCE {
                violations.push(ConstraintViolation {
                    constraint_id: constraint.id.clone(),
                    description,
                    magnitude,
                    hard: constraint.hard,
                });
            }
        }

        violations
    }

    /// Exhaustively evaluates every assignment of the compiled model.
    ///
    /// This is a reference implementation used to validate compilations and to
    /// check sampler output on small instances. It is bounded by
    /// [`QuboCompiler::max_exhaustive_variables`].
    pub fn brute_force(&self) -> Result<OptimizationSolution> {
        let variables = &self.bqm.variables;
        if variables.len() > self.max_exhaustive_variables {
            return Err(OptimizationError::ProblemTooLarge {
                variables: variables.len(),
                limit: self.max_exhaustive_variables,
            });
        }

        let mut best: Option<(f64, BTreeMap<String, u8>)> = None;
        for mask in 0u64..(1u64 << variables.len()) {
            let sample: BTreeMap<String, u8> = variables
                .iter()
                .enumerate()
                .map(|(index, name)| (name.clone(), u8::from(mask >> index & 1 == 1)))
                .collect();
            let energy = self.bqm.energy(&sample);
            if best
                .as_ref()
                .is_none_or(|(best_energy, _)| energy < *best_energy)
            {
                best = Some((energy, sample));
            }
        }

        let (_, sample) = best.expect("a model with variables has at least one assignment");
        Ok(self.decode(&sample))
    }
}

/// Accumulates QUBO coefficients while constraints are compiled.
struct QuboBuilder {
    declared: Vec<String>,
    slack_variables: Vec<SlackVariable>,
    linear: BTreeMap<String, f64>,
    quadratic: BTreeMap<(String, String), f64>,
    offset: f64,
}

impl QuboBuilder {
    fn new(declared: &[String]) -> Self {
        Self {
            declared: declared.to_vec(),
            slack_variables: Vec::new(),
            linear: declared.iter().map(|name| (name.clone(), 0.0)).collect(),
            quadratic: BTreeMap::new(),
            offset: 0.0,
        }
    }

    fn check_declared(&self, constraint_id: &str, variable: &str) -> Result<()> {
        if self.linear.contains_key(variable) {
            return Ok(());
        }
        Err(OptimizationError::UnknownVariable {
            constraint_id: constraint_id.to_string(),
            variable: variable.to_string(),
        })
    }

    fn declare_slack(&mut self, slack: SlackVariable) {
        self.linear.entry(slack.name.clone()).or_insert(0.0);
        self.slack_variables.push(slack);
    }

    fn add_linear(&mut self, variable: &str, coefficient: f64) {
        *self.linear.entry(variable.to_string()).or_insert(0.0) += coefficient;
    }

    fn add_quadratic(&mut self, first: &str, second: &str, coefficient: f64) {
        if first == second {
            // A binary variable squared is itself.
            self.add_linear(first, coefficient);
            return;
        }
        let key = if first <= second {
            (first.to_string(), second.to_string())
        } else {
            (second.to_string(), first.to_string())
        };
        *self.quadratic.entry(key).or_insert(0.0) += coefficient;
    }

    fn add_offset(&mut self, offset: f64) {
        self.offset += offset;
    }

    /// Adds `weight * (sum(coefficient * variable) + constant)^2`, using the
    /// binary identity `x^2 == x`.
    fn add_square(&mut self, terms: &[(String, f64)], constant: f64, weight: f64) {
        for (index, (name, coefficient)) in terms.iter().enumerate() {
            self.add_linear(
                name,
                weight * (coefficient * coefficient + 2.0 * coefficient * constant),
            );
            for (other, other_coefficient) in terms.iter().skip(index + 1) {
                self.add_quadratic(name, other, weight * 2.0 * coefficient * other_coefficient);
            }
        }
        self.add_offset(weight * constant * constant);
    }

    /// Sum of absolute objective coefficients, used to size default penalties
    /// so that violating a constraint can never pay for itself.
    fn magnitude(&self) -> f64 {
        let linear: f64 = self.linear.values().map(|value| value.abs()).sum();
        let quadratic: f64 = self.quadratic.values().map(|value| value.abs()).sum();
        1.0 + linear + quadratic
    }

    fn finish(self) -> (BinaryQuadraticModel, Vec<SlackVariable>) {
        let mut variables = self.declared.clone();
        variables.extend(self.slack_variables.iter().map(|slack| slack.name.clone()));

        let linear = variables
            .iter()
            .map(|name| LinearTerm::new(name, self.linear.get(name).copied().unwrap_or_default()))
            .collect();
        let quadratic = self
            .quadratic
            .into_iter()
            .filter(|(_, coefficient)| coefficient.abs() > INTEGRALITY_TOLERANCE)
            .map(|((first, second), coefficient)| QuadraticTerm::new(first, second, coefficient))
            .collect();

        (
            BinaryQuadraticModel {
                variables,
                linear,
                quadratic,
                offset: self.offset,
            },
            self.slack_variables,
        )
    }
}

fn unit_terms(
    builder: &QuboBuilder,
    constraint_id: &str,
    variables: &[String],
) -> Result<Vec<(String, f64)>> {
    variables
        .iter()
        .map(|variable| {
            builder.check_declared(constraint_id, variable)?;
            Ok((variable.clone(), 1.0))
        })
        .collect()
}

fn weighted_terms(
    builder: &QuboBuilder,
    constraint_id: &str,
    terms: &[LinearTerm],
) -> Result<Vec<(String, f64)>> {
    let mut merged: Vec<(String, f64)> = Vec::with_capacity(terms.len());
    for term in terms {
        builder.check_declared(constraint_id, &term.variable)?;
        match merged.iter_mut().find(|(name, _)| name == &term.variable) {
            Some((_, coefficient)) => *coefficient += term.coefficient,
            None => merged.push((term.variable.clone(), term.coefficient)),
        }
    }
    Ok(merged)
}

fn require_integral(constraint_id: &str, terms: &[(String, f64)], rhs: f64) -> Result<()> {
    let non_integral = terms
        .iter()
        .map(|(_, coefficient)| *coefficient)
        .chain(std::iter::once(rhs))
        .find(|value| (value - value.round()).abs() > INTEGRALITY_TOLERANCE);

    match non_integral {
        Some(value) => Err(OptimizationError::UnsupportedConstraint {
            constraint_id: constraint_id.to_string(),
            reason: format!(
                "inequality coefficients must be integral for slack encoding, found {value}; scale the units first"
            ),
        }),
        None => Ok(()),
    }
}

/// Binary weights that can represent every integer in `0..=bound`.
fn slack_weights(bound: u64) -> Vec<u64> {
    let mut weights = Vec::new();
    let mut remaining = bound;
    let mut weight = 1u64;
    while remaining >= weight {
        weights.push(weight);
        remaining -= weight;
        weight *= 2;
    }
    if remaining > 0 {
        weights.push(remaining);
    }
    weights
}

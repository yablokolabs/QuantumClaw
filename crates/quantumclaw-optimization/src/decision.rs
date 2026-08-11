use crate::error::{OptimizationError, Result};
use quantumclaw_ir::optimization::{BinaryVariable, OptimizationConstraint, OptimizationProblem};
use quantumclaw_ir::DecisionProblem;

/// Returns the optimization model a solver should work on.
///
/// Uses the explicit model when the decision problem carries one, and otherwise
/// derives the generic action-selection model.
pub fn optimization_problem_for(problem: &DecisionProblem) -> Result<OptimizationProblem> {
    match &problem.optimization {
        Some(optimization) => Ok(optimization.clone()),
        None => action_selection_problem(problem),
    }
}

/// Derives a generic action-selection model from a decision problem.
///
/// One binary variable per candidate action, an objective of `utility - risk`
/// to maximize, and dependencies compiled into implication constraints: an
/// action that depends on another cannot be selected without it.
///
/// Free-text decision constraints have no variable structure, so they are
/// carried into the model metadata rather than silently dropped.
pub fn action_selection_problem(problem: &DecisionProblem) -> Result<OptimizationProblem> {
    if problem.candidate_actions.is_empty() {
        return Err(OptimizationError::UnsupportedDecisionProblem {
            problem_id: problem.id.clone(),
            reason: "it declares no candidate actions and carries no explicit optimization model"
                .into(),
        });
    }

    let mut model = OptimizationProblem::maximize(format!("{}-action-selection", problem.id))
        .with_metadata("source", "decision-problem")
        .with_metadata("decision_problem_id", problem.id.clone());

    for action in &problem.candidate_actions {
        let mut variable = BinaryVariable::new(action.id.clone())
            .with_metadata("action", action.id.clone())
            .with_metadata("name", action.name.clone());
        if let Some(tool_hint) = &action.tool_hint {
            variable = variable.with_metadata("tool_hint", tool_hint.clone());
        }
        model.variables.push(variable);
        model
            .linear
            .push(quantumclaw_ir::optimization::LinearTerm::new(
                action.id.clone(),
                action.utility.0 - action.risk.score,
            ));
    }

    for dependency in &problem.dependencies {
        let known = |id: &str| {
            problem
                .candidate_actions
                .iter()
                .any(|action| action.id == id)
        };
        if !known(&dependency.before) || !known(&dependency.after) {
            continue;
        }
        model = model.with_constraint(OptimizationConstraint::implication(
            format!("dependency-{}-{}", dependency.before, dependency.after),
            dependency.after.clone(),
            dependency.before.clone(),
        ));
    }

    for constraint in &problem.constraints {
        model = model.with_metadata(
            format!("decision_constraint.{}", constraint.id),
            constraint.description.clone(),
        );
    }

    Ok(model)
}

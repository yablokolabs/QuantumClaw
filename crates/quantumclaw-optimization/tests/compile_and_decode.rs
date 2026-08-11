//! Behavioral tests for the domain-neutral optimization layer.
//!
//! Every test drives the public compile -> solve -> decode path on an instance
//! small enough that the optimum can be computed by hand.

use quantumclaw_ir::optimization::{
    LinearTerm, OptimizationConstraint, OptimizationProblem, OptimizationSolution,
};
use quantumclaw_ir::{CandidateAction, DecisionProblem, Dependency};
use quantumclaw_optimization::{action_selection_problem, optimization_problem_for, QuboCompiler};
use std::collections::BTreeMap;

fn sample(pairs: &[(&str, u8)]) -> BTreeMap<String, u8> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), *value))
        .collect()
}

/// Three deliveries assigned to two vehicles. Costs are chosen so the optimum
/// is unambiguous: d1 -> v1 (2 vs 9), d2 -> v2 (1 vs 8), d3 -> v1 (3 vs 7).
fn assignment_problem() -> OptimizationProblem {
    OptimizationProblem::minimize("assignment")
        .with_term("d1_v1", 2.0)
        .with_term("d1_v2", 9.0)
        .with_term("d2_v1", 8.0)
        .with_term("d2_v2", 1.0)
        .with_term("d3_v1", 3.0)
        .with_term("d3_v2", 7.0)
        .with_constraint(OptimizationConstraint::exactly_one(
            "d1-assigned",
            ["d1_v1", "d1_v2"],
        ))
        .with_constraint(OptimizationConstraint::exactly_one(
            "d2-assigned",
            ["d2_v1", "d2_v2"],
        ))
        .with_constraint(OptimizationConstraint::exactly_one(
            "d3-assigned",
            ["d3_v1", "d3_v2"],
        ))
}

#[test]
fn exhaustive_search_over_compiled_model_finds_the_hand_computed_assignment() {
    let compiled = QuboCompiler::default()
        .compile(&assignment_problem())
        .expect("assignment problem compiles");

    let best = compiled.brute_force().expect("model is small enough");

    assert_eq!(best.selected, vec!["d1_v1", "d2_v2", "d3_v1"]);
    assert!(best.feasible, "optimum must satisfy every hard constraint");
    assert!(best.violations.is_empty());
    assert!(
        (best.objective_value - 6.0).abs() < 1e-9,
        "objective must be reported in problem units, got {}",
        best.objective_value
    );
}

#[test]
fn maximization_objective_is_reported_in_original_units() {
    // Selecting both is worth 1.5; the pairwise term makes it worth 0.9, so the
    // best choice is the single high-utility variable.
    let problem = OptimizationProblem::maximize("selection")
        .with_term("a", 1.0)
        .with_term("b", 0.5)
        .with_interaction("a", "b", -0.6);

    let compiled = QuboCompiler::default()
        .compile(&problem)
        .expect("problem compiles");
    let best = compiled.brute_force().expect("model is small enough");

    assert_eq!(best.selected, vec!["a"]);
    assert!(
        (best.objective_value - 1.0).abs() < 1e-9,
        "maximization must report +1.0, not a negated energy, got {}",
        best.objective_value
    );
}

#[test]
fn capacity_overload_decodes_as_infeasible_and_names_the_constraint() {
    // Two deliveries of 6 units each on one vehicle with capacity 10.
    let problem = OptimizationProblem::minimize("capacity")
        .with_term("d1_v1", -1.0)
        .with_term("d2_v1", -1.0)
        .with_constraint(OptimizationConstraint::linear_at_most(
            "v1-capacity",
            vec![LinearTerm::new("d1_v1", 6.0), LinearTerm::new("d2_v1", 6.0)],
            10.0,
        ));

    let compiled = QuboCompiler::default()
        .compile(&problem)
        .expect("capacity problem compiles");

    let overloaded = compiled.decode(&sample(&[("d1_v1", 1), ("d2_v1", 1)]));
    assert!(!overloaded.feasible);
    let violation = overloaded
        .violations
        .iter()
        .find(|violation| violation.constraint_id == "v1-capacity")
        .expect("capacity violation is reported");
    assert!(
        (violation.magnitude - 2.0).abs() < 1e-9,
        "12 units against a capacity of 10 overshoots by 2, got {}",
        violation.magnitude
    );

    let within_capacity = compiled.decode(&sample(&[("d1_v1", 1), ("d2_v1", 0)]));
    assert!(within_capacity.feasible);
    assert!(within_capacity.violations.is_empty());
}

#[test]
fn capacity_constraint_keeps_the_optimum_within_capacity() {
    // Both deliveries are individually attractive, but only one fits.
    let problem = OptimizationProblem::minimize("capacity-optimum")
        .with_term("d1_v1", -5.0)
        .with_term("d2_v1", -4.0)
        .with_constraint(OptimizationConstraint::linear_at_most(
            "v1-capacity",
            vec![LinearTerm::new("d1_v1", 6.0), LinearTerm::new("d2_v1", 6.0)],
            10.0,
        ));

    let best = QuboCompiler::default()
        .compile(&problem)
        .expect("problem compiles")
        .brute_force()
        .expect("model is small enough");

    assert_eq!(best.selected, vec!["d1_v1"]);
    assert!(best.feasible);
}

#[test]
fn raising_the_penalty_widens_the_energy_gap_for_a_violating_sample() {
    let build = |penalty: f64| {
        OptimizationProblem::minimize("penalty-scaling")
            .with_term("a", -1.0)
            .with_term("b", -1.0)
            .with_constraint(
                OptimizationConstraint::at_most_one("only-one", ["a", "b"]).with_penalty(penalty),
            )
    };

    let violating = sample(&[("a", 1), ("b", 1)]);
    let feasible = sample(&[("a", 1), ("b", 0)]);

    let gap = |penalty: f64| {
        let compiled = QuboCompiler::default()
            .compile(&build(penalty))
            .expect("problem compiles");
        compiled.decode(&violating).energy - compiled.decode(&feasible).energy
    };

    let small = gap(1.0);
    let large = gap(50.0);
    assert!(
        large > small + 40.0,
        "a heavier penalty must make the violating sample far worse: {small} vs {large}"
    );
}

#[test]
fn exhaustive_search_respects_an_implication_constraint() {
    // Alone, `deploy` is the best pick. The implication drags in `review`,
    // which costs a little, and the pair still beats doing nothing.
    let base = OptimizationProblem::maximize("implication")
        .with_term("deploy", 1.0)
        .with_term("review", -0.2);

    let without = QuboCompiler::default()
        .compile(&base)
        .expect("problem compiles")
        .brute_force()
        .expect("model is small enough");
    assert_eq!(without.selected, vec!["deploy"]);

    let with_implication = QuboCompiler::default()
        .compile(&base.with_constraint(OptimizationConstraint::implication(
            "deploy-needs-review",
            "deploy",
            "review",
        )))
        .expect("problem compiles")
        .brute_force()
        .expect("model is small enough");

    assert_eq!(with_implication.selected, vec!["deploy", "review"]);
    assert!(with_implication.feasible);
    assert!(
        (with_implication.objective_value - 0.8).abs() < 1e-9,
        "objective must exclude penalties, got {}",
        with_implication.objective_value
    );
}

#[test]
fn at_least_one_constraint_forces_a_selection_the_objective_would_avoid() {
    // Every variable costs something, so an unconstrained minimum selects none.
    let problem = OptimizationProblem::minimize("coverage")
        .with_term("a", 3.0)
        .with_term("b", 5.0)
        .with_constraint(OptimizationConstraint::at_least_one("cover", ["a", "b"]));

    let best = QuboCompiler::default()
        .compile(&problem)
        .expect("problem compiles")
        .brute_force()
        .expect("model is small enough");

    assert_eq!(best.selected, vec!["a"]);
    assert!(best.feasible);
    assert!((best.objective_value - 3.0).abs() < 1e-9);
}

#[test]
fn soft_constraint_violations_are_reported_without_marking_the_solution_infeasible() {
    let problem = OptimizationProblem::minimize("soft")
        .with_term("a", -1.0)
        .with_term("b", -1.0)
        .with_constraint(
            OptimizationConstraint::at_most_one("prefer-one", ["a", "b"])
                .with_penalty(0.5)
                .soft(),
        );

    let compiled = QuboCompiler::default()
        .compile(&problem)
        .expect("problem compiles");
    let both = compiled.decode(&sample(&[("a", 1), ("b", 1)]));

    assert!(both.feasible, "soft constraints do not decide feasibility");
    assert_eq!(both.violations.len(), 1);
    assert!(!both.violations[0].hard);
}

#[test]
fn decision_problem_without_an_explicit_model_optimizes_candidate_actions() {
    let mut problem = DecisionProblem::new("release");
    problem.candidate_actions = vec![
        CandidateAction::new("test", "Run tests", "Validate before release").with_utility(0.9),
        CandidateAction::new("deploy", "Deploy", "Ship the release").with_utility(0.8),
        CandidateAction::new("skip-checks", "Skip checks", "Bypass validation")
            .with_utility(0.2)
            .with_risk(0.95),
    ];
    problem.dependencies = vec![Dependency::new(
        "test",
        "deploy",
        "Deploying depends on tests",
    )];

    let model = optimization_problem_for(&problem).expect("candidate actions are optimizable");
    let best = QuboCompiler::default()
        .compile(&model)
        .expect("derived model compiles")
        .brute_force()
        .expect("model is small enough");

    assert!(best.selected.contains(&"test".to_string()));
    assert!(best.selected.contains(&"deploy".to_string()));
    assert!(
        !best.selected.contains(&"skip-checks".to_string()),
        "an action whose risk exceeds its utility must not be selected"
    );
}

#[test]
fn an_explicit_model_on_a_decision_problem_wins_over_the_derived_one() {
    let explicit = OptimizationProblem::minimize("explicit").with_term("only-me", -1.0);
    let mut problem = DecisionProblem::new("mixed");
    problem.candidate_actions = vec![CandidateAction::new("ignored", "Ignored", "Not used")];
    let problem = problem.with_optimization(explicit);

    let model = optimization_problem_for(&problem).expect("explicit model is used");

    assert_eq!(model.id, "explicit");
    assert_eq!(model.variable_names(), vec!["only-me"]);
}

#[test]
fn a_decision_problem_with_nothing_to_optimize_is_rejected() {
    let error = optimization_problem_for(&DecisionProblem::new("empty"))
        .expect_err("an empty decision problem cannot be optimized");

    assert!(
        error.to_string().contains("empty"),
        "error must name the problem: {error}"
    );
}

#[test]
fn dependencies_between_candidate_actions_become_implication_constraints() {
    // `deploy` is attractive, `test` is not, but the dependency forces it in.
    let mut problem = DecisionProblem::new("dependency");
    problem.candidate_actions = vec![
        CandidateAction::new("test", "Run tests", "Slow but required")
            .with_utility(0.1)
            .with_risk(0.3),
        CandidateAction::new("deploy", "Deploy", "High value").with_utility(0.95),
    ];
    problem.dependencies = vec![Dependency::new("test", "deploy", "Deploy needs tests")];

    let best: OptimizationSolution = QuboCompiler::default()
        .compile(&action_selection_problem(&problem).expect("model is derived"))
        .expect("derived model compiles")
        .brute_force()
        .expect("model is small enough");

    assert_eq!(best.selected, vec!["test", "deploy"]);
}

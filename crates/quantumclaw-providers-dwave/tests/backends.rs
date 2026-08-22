//! Behavioral tests for the D-Wave provider.
//!
//! Tests that need Ocean are skipped unless `QUANTUMCLAW_DWAVE_PYTHON` points
//! at an interpreter that can import the bridge. Setting
//! `QUANTUMCLAW_DWAVE_REQUIRE=1` turns those skips into failures so CI can
//! prove the Ocean lane really ran.

use quantumclaw_core::{AgentTask, SolverBackend, SolverContext, SolverKind};
use quantumclaw_ir::optimization::{OptimizationConstraint, OptimizationProblem};
use quantumclaw_ir::DecisionProblem;
use quantumclaw_providers_dwave::models::{BridgeBackend, BridgeBqm, BridgeRequest};
use quantumclaw_providers_dwave::{
    DWaveBridge, DWaveConfig, DWaveExactSolverBackend, DWaveLeapHybridBackend, DWaveQpuBackend,
    DWaveRunMetadata, DWaveSimulatedAnnealingBackend, DWaveSimulatedQuantumAnnealingBackend,
    ExactParams, SimulatedAnnealingParams, SimulatedQuantumAnnealingParams,
};
use std::sync::Arc;
use std::time::Duration;

/// Three deliveries, two vehicles, one obvious best assignment: d1->v1 (2),
/// d2->v2 (1), d3->v1 (3), for a total cost of 6.
fn assignment_problem() -> DecisionProblem {
    let model = OptimizationProblem::minimize("fleet-assignment")
        .with_term("d1_v1", 2.0)
        .with_term("d1_v2", 9.0)
        .with_term("d2_v1", 8.0)
        .with_term("d2_v2", 1.0)
        .with_term("d3_v1", 3.0)
        .with_term("d3_v2", 7.0)
        .with_constraint(OptimizationConstraint::exactly_one(
            "d1",
            ["d1_v1", "d1_v2"],
        ))
        .with_constraint(OptimizationConstraint::exactly_one(
            "d2",
            ["d2_v1", "d2_v2"],
        ))
        .with_constraint(OptimizationConstraint::exactly_one(
            "d3",
            ["d3_v1", "d3_v2"],
        ));
    DecisionProblem::new("fleet").with_optimization(model)
}

fn context() -> SolverContext {
    SolverContext::from_task(&AgentTask::new("assign deliveries to vehicles"))
}

/// A bridge that cannot start, to prove failures stay actionable.
fn broken_bridge() -> Arc<DWaveBridge> {
    Arc::new(DWaveBridge::new(
        DWaveConfig::default().with_python("quantumclaw-no-such-interpreter"),
    ))
}

/// A real subprocess that ignores its input and emits a fixed bridge response.
/// This exercises the protocol boundary rather than a mocked client.
fn canned_bridge(response: &str) -> DWaveBridge {
    DWaveBridge::new(
        DWaveConfig::default()
            .with_python("/bin/sh")
            .with_bridge_args([
                "-c".to_string(),
                "cat >/dev/null; printf '%s' \"$0\"".to_string(),
                response.to_string(),
            ])
            .with_timeout(Duration::from_secs(10)),
    )
}

fn sample_request() -> BridgeRequest {
    BridgeRequest::new(
        BridgeBackend::SimulatedAnnealing,
        "canned",
        BridgeBqm {
            variables: vec!["a".into()],
            linear: vec![("a".into(), -1.0)],
            quadratic: Vec::new(),
            offset: 0.0,
        },
    )
}

/// Interpreter that can run the bridge, or `None` when this host cannot.
fn ocean_python() -> Option<String> {
    let python = std::env::var("QUANTUMCLAW_DWAVE_PYTHON")
        .ok()
        .filter(|value| !value.is_empty());
    match (
        python,
        std::env::var("QUANTUMCLAW_DWAVE_REQUIRE").as_deref(),
    ) {
        (Some(python), _) => Some(python),
        (None, Ok("1")) => {
            panic!("QUANTUMCLAW_DWAVE_REQUIRE=1 but QUANTUMCLAW_DWAVE_PYTHON is not set")
        }
        (None, _) => {
            eprintln!(
                "skipping: set QUANTUMCLAW_DWAVE_PYTHON to an interpreter with Ocean installed"
            );
            None
        }
    }
}

fn ocean_bridge() -> Option<Arc<DWaveBridge>> {
    ocean_python().map(|python| {
        Arc::new(DWaveBridge::new(
            DWaveConfig::default()
                .with_python(python)
                .with_timeout(Duration::from_secs(120)),
        ))
    })
}

#[tokio::test]
async fn a_missing_ocean_installation_explains_how_to_install_it() {
    let backend = DWaveSimulatedAnnealingBackend::new(broken_bridge());

    let error = backend
        .solve(assignment_problem(), context())
        .await
        .map(|_| ())
        .expect_err("a missing interpreter cannot solve");

    // The guidance has to be a command the user can actually run, not just a
    // statement that something is missing.
    let message = error.to_string();
    assert!(
        message.contains("pip install") && message.contains("quantumclaw-dwave"),
        "expected a runnable install command, got: {message}"
    );
    assert!(
        message.contains("ocean_missing"),
        "expected the typed code in the wrapped error, got: {message}"
    );
}

#[tokio::test]
async fn provider_error_codes_survive_the_bridge_boundary() {
    let cases = [
        ("missing_credentials", "missing_credentials"),
        ("authentication_failed", "authentication_failed"),
        ("embedding_failed", "embedding_failed"),
        ("solver_unavailable", "solver_unavailable"),
        ("invalid_bqm", "invalid_bqm"),
        ("problem_too_large", "problem_too_large"),
        ("no_feasible_result", "no_feasible_result"),
        ("something_new_from_ocean", "sampler_failed"),
    ];

    for (reported, expected) in cases {
        let response = format!(
            r#"{{"ok": false, "error": {{"code": "{reported}", "message": "provider said no", "cause": "OceanException: detail"}}}}"#
        );
        let error = canned_bridge(&response)
            .execute(&sample_request())
            .await
            .err()
            .unwrap_or_else(|| panic!("{reported} must fail"));

        assert_eq!(error.code(), expected, "for reported code {reported}");
        assert!(
            error.to_string().contains("OceanException: detail"),
            "the underlying cause must survive: {error}"
        );
    }
}

#[tokio::test]
async fn unparseable_bridge_output_is_reported_as_a_protocol_error() {
    let error = canned_bridge("this is not json")
        .execute(&sample_request())
        .await
        .map(|_| ())
        .expect_err("garbage output cannot be decoded");

    assert_eq!(error.code(), "bridge_protocol_error");
    assert!(error.to_string().contains("this is not json"));
}

#[tokio::test]
async fn a_bridge_that_never_finishes_is_killed_and_reported_as_a_timeout() {
    let bridge = DWaveBridge::new(
        DWaveConfig::default()
            .with_python("/bin/sh")
            .with_bridge_args(["-c".to_string(), "sleep 30".to_string()])
            .with_timeout(Duration::from_millis(250)),
    );

    let error = bridge
        .execute(&sample_request())
        .await
        .map(|_| ())
        .expect_err("a hanging bridge must not hang the caller");

    assert_eq!(error.code(), "timeout");
}

#[tokio::test]
async fn the_exact_backend_refuses_oversized_problems_without_starting_ocean() {
    // The interpreter does not exist, so reaching the bridge would report
    // ocean_missing. Getting the size refusal proves the guard ran first.
    let backend = DWaveExactSolverBackend::new(broken_bridge())
        .with_params(ExactParams::default().with_max_variables(2));

    let error = backend
        .solve(assignment_problem(), context())
        .await
        .map(|_| ())
        .expect_err("six variables exceed a limit of two");

    let message = error.to_string();
    assert!(message.contains("problem_too_large"), "got: {message}");
    assert!(message.contains("at most 2"), "got: {message}");
}

#[test]
fn backend_kinds_describe_the_hardware_honestly() {
    let bridge = broken_bridge();

    assert_eq!(
        DWaveSimulatedAnnealingBackend::new(bridge.clone()).kind(),
        SolverKind::Classical,
        "simulated annealing runs on classical hardware"
    );
    assert_eq!(
        DWaveExactSolverBackend::new(bridge.clone()).kind(),
        SolverKind::Classical
    );
    assert_eq!(
        DWaveSimulatedQuantumAnnealingBackend::new(bridge.clone()).kind(),
        SolverKind::QuantumInspired,
        "path-integral annealing emulates quantum dynamics on a local CPU; it is never a QPU"
    );
    assert_eq!(
        DWaveLeapHybridBackend::new(bridge.clone()).kind(),
        SolverKind::QuantumHybrid
    );
    assert_eq!(
        DWaveQpuBackend::new(bridge).kind(),
        SolverKind::QuantumAnnealing
    );
}

#[test]
fn only_remote_backends_declare_that_they_need_credentials() {
    let bridge = broken_bridge();

    assert!(
        !DWaveSimulatedAnnealingBackend::new(bridge.clone())
            .capabilities()
            .requires_credentials
    );
    assert!(
        !DWaveExactSolverBackend::new(bridge.clone())
            .capabilities()
            .requires_credentials
    );
    assert!(
        !DWaveSimulatedQuantumAnnealingBackend::new(bridge.clone())
            .capabilities()
            .requires_credentials,
        "the local emulator needs no Leap account"
    );
    assert!(
        DWaveLeapHybridBackend::new(bridge.clone())
            .capabilities()
            .requires_credentials
    );
    assert!(
        DWaveQpuBackend::new(bridge)
            .capabilities()
            .requires_credentials
    );
}

#[tokio::test]
async fn exhaustive_ocean_search_returns_the_hand_computed_optimum() {
    let Some(bridge) = ocean_bridge() else {
        return;
    };

    let output = DWaveExactSolverBackend::new(bridge)
        .solve(assignment_problem(), context())
        .await
        .expect("the exact solver runs");

    let solution = output.solution.expect("an optimization result is returned");
    assert_eq!(solution.selected, vec!["d1_v1", "d2_v2", "d3_v1"]);
    assert!(solution.feasible);
    assert!((solution.objective_value - 6.0).abs() < 1e-9);
}

#[tokio::test]
async fn simulated_quantum_annealing_matches_the_exhaustive_optimum() {
    let Some(bridge) = ocean_bridge() else {
        return;
    };

    let output = DWaveSimulatedQuantumAnnealingBackend::new(bridge)
        .with_params(
            SimulatedQuantumAnnealingParams::default()
                .with_num_reads(200)
                .with_num_sweeps(500)
                .with_seed(42),
        )
        .solve(assignment_problem(), context())
        .await
        .expect("the quantum annealing emulator runs");

    let solution = output.solution.expect("an optimization result is returned");
    assert_eq!(solution.selected, vec!["d1_v1", "d2_v2", "d3_v1"]);
    assert!(solution.feasible);
    assert!(
        output.steps.len() == 3,
        "each selection becomes a plan step"
    );
}

#[tokio::test]
async fn simulated_annealing_matches_the_exhaustive_optimum() {
    let Some(bridge) = ocean_bridge() else {
        return;
    };

    let output = DWaveSimulatedAnnealingBackend::new(bridge)
        .with_params(
            SimulatedAnnealingParams::default()
                .with_num_reads(200)
                .with_num_sweeps(500)
                .with_seed(42),
        )
        .solve(assignment_problem(), context())
        .await
        .expect("simulated annealing runs");

    let solution = output.solution.expect("an optimization result is returned");
    assert_eq!(solution.selected, vec!["d1_v1", "d2_v2", "d3_v1"]);
    assert!(solution.feasible);
    assert!(
        output.steps.len() == 3,
        "each selection becomes a plan step"
    );
}

#[tokio::test]
async fn a_seeded_run_is_reproducible() {
    let Some(bridge) = ocean_bridge() else {
        return;
    };
    let backend = DWaveSimulatedAnnealingBackend::new(bridge).with_params(
        SimulatedAnnealingParams::default()
            .with_num_reads(25)
            .with_seed(1234),
    );

    let first = backend
        .solve(assignment_problem(), context())
        .await
        .expect("first run");
    let second = backend
        .solve(assignment_problem(), context())
        .await
        .expect("second run");

    assert_eq!(
        first.solution.unwrap().assignments,
        second.solution.unwrap().assignments
    );
}

#[tokio::test]
async fn a_run_reports_provider_metadata_without_inventing_qpu_measurements() {
    let Some(bridge) = ocean_bridge() else {
        return;
    };

    let output = DWaveSimulatedAnnealingBackend::new(bridge)
        .solve(assignment_problem(), context())
        .await
        .expect("simulated annealing runs");

    assert_eq!(output.telemetry.provider.as_deref(), Some("dwave"));
    let metadata =
        DWaveRunMetadata::from_telemetry(&output.telemetry).expect("provider metadata is attached");

    assert_eq!(metadata.backend, "simulated_annealing");
    assert!(metadata.sampler.contains("SimulatedAnnealingSampler"));
    assert_eq!(metadata.variables, 6);
    assert!(metadata.solver_runtime_ms.is_some());
    assert!(
        metadata.qpu_access_time_us.is_none(),
        "a classical sampler must not report QPU timings"
    );
    assert!(metadata.chain_break_fraction.is_none());
}

#[tokio::test]
async fn cloud_backends_report_missing_credentials_instead_of_hanging() {
    let Some(bridge) = ocean_bridge() else {
        return;
    };
    if std::env::var("DWAVE_API_TOKEN").is_ok() {
        eprintln!("skipping: a Leap token is present, so this host can reach the cloud");
        return;
    }

    let error = DWaveLeapHybridBackend::new(bridge)
        .solve(assignment_problem(), context())
        .await
        .map(|_| ())
        .expect_err("no credentials means no hybrid run");

    let message = error.to_string();
    assert!(
        message.contains("missing_credentials") || message.contains("authentication_failed"),
        "got: {message}"
    );
    assert!(
        message.contains("DWAVE_API_TOKEN") || message.contains("dwave config"),
        "the message should say how to fix it: {message}"
    );
}

#[tokio::test]
async fn probing_reports_which_lanes_this_host_can_run() {
    let Some(bridge) = ocean_bridge() else {
        return;
    };

    let report = bridge.probe().await.expect("the probe runs");

    assert!(report.supports(BridgeBackend::SimulatedAnnealing));
    assert!(report.supports(BridgeBackend::Exact));
    assert!(
        report.supports(BridgeBackend::SimulatedQuantumAnnealing),
        "the local extra provides the path-integral annealing sampler"
    );
}

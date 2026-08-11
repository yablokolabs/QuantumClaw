//! The registry is how the CLI and agents pick a backend by name.

use async_trait::async_trait;
use quantumclaw_core::{
    Result, SolverBackend, SolverContext, SolverKind, SolverOutput, SolverRegistry, SolverScore,
};
use quantumclaw_ir::DecisionProblem;
use std::sync::Arc;

struct StubBackend {
    name: &'static str,
    kind: SolverKind,
}

#[async_trait]
impl SolverBackend for StubBackend {
    fn name(&self) -> &'static str {
        self.name
    }

    fn kind(&self) -> SolverKind {
        self.kind
    }

    async fn solve(
        &self,
        _problem: DecisionProblem,
        _context: SolverContext,
    ) -> Result<SolverOutput> {
        Ok(SolverOutput {
            backend: self.name.into(),
            backend_kind: self.kind,
            steps: Vec::new(),
            score: SolverScore::default(),
            rationale: String::new(),
            telemetry: quantumclaw_core::BackendTelemetry::new(self.name, self.kind),
            solution: None,
        })
    }
}

fn registry() -> SolverRegistry {
    let mut registry = SolverRegistry::new();
    registry.register(Arc::new(StubBackend {
        name: "greedy-classical",
        kind: SolverKind::Classical,
    }));
    registry.register_as(
        "dwave-sa",
        Arc::new(StubBackend {
            name: "dwave-simulated-annealing",
            kind: SolverKind::Classical,
        }),
    );
    registry
}

#[tokio::test]
async fn a_backend_registered_under_an_alias_is_resolved_by_that_alias() {
    let resolved = registry()
        .get("dwave-sa")
        .expect("alias resolves to a backend");

    assert_eq!(resolved.name(), "dwave-simulated-annealing");
}

#[test]
fn resolving_an_unknown_backend_reports_the_available_names() {
    let error = registry()
        .require("dwave-qpu")
        .map(|_| ())
        .expect_err("unregistered backend cannot be resolved");

    let message = error.to_string();
    assert!(
        message.contains("dwave-qpu"),
        "names the request: {message}"
    );
    assert!(
        message.contains("dwave-sa") && message.contains("greedy-classical"),
        "lists what is available: {message}"
    );
}

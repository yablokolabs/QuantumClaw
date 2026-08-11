//! Intent routing and the typed/JSON equivalence of a brain.

use async_trait::async_trait;
use quantumclaw_brains::{
    BrainCapabilities, BrainMatch, BrainOperation, BrainPlan, BrainRegistry, BrainSolveContext,
    Decomposition, ErasedBrain, Explanation, Formulation, JsonBrain, KpiReport, QuantumBrain,
    ValidationReport,
};
use quantumclaw_core::{AgentTask, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CountingInput {
    items: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CountingOutput {
    total: usize,
}

/// A minimal brain: enough to exercise routing and the JSON boundary.
struct CountingBrain {
    id: String,
    keywords: Vec<String>,
}

#[async_trait]
impl QuantumBrain for CountingBrain {
    type Input = CountingInput;
    type Output = CountingOutput;

    fn id(&self) -> &str {
        &self.id
    }

    fn capabilities(&self) -> BrainCapabilities {
        BrainCapabilities::new(&self.id, "counting")
    }

    fn can_handle(&self, task: &AgentTask) -> BrainMatch {
        BrainMatch::from_keywords(&task.description, &self.keywords)
    }

    async fn validate(&self, input: &Self::Input) -> Result<ValidationReport> {
        let mut report = ValidationReport::default();
        if input.items.is_empty() {
            report.error("items", "at least one item is required");
        }
        Ok(report)
    }

    async fn plan(&self, _input: &Self::Input) -> Result<BrainPlan> {
        Ok(BrainPlan::default())
    }

    async fn formulate(&self, _input: &Self::Input) -> Result<Vec<Formulation>> {
        Ok(Vec::new())
    }

    async fn decompose(&self, _input: &Self::Input) -> Result<Decomposition> {
        Ok(Decomposition::single_block("counting"))
    }

    async fn solve(&self, input: Self::Input, _context: BrainSolveContext) -> Result<Self::Output> {
        Ok(CountingOutput {
            total: input.items.len(),
        })
    }

    async fn evaluate(&self, output: &Self::Output) -> Result<KpiReport> {
        let mut report = KpiReport::default();
        report.set("total", output.total as f64);
        Ok(report)
    }

    async fn explain(&self, output: &Self::Output) -> Result<Explanation> {
        Ok(Explanation::new(format!("counted {} items", output.total)))
    }
}

fn brain(id: &str, keywords: &[&str]) -> Arc<CountingBrain> {
    Arc::new(CountingBrain {
        id: id.to_string(),
        keywords: keywords.iter().map(|word| word.to_string()).collect(),
    })
}

fn registry() -> BrainRegistry {
    let mut registry = BrainRegistry::new();
    registry.register(Arc::new(JsonBrain::new(brain(
        "counter",
        &["count", "tally"],
    ))));
    registry.register(Arc::new(JsonBrain::new(brain(
        "router",
        &["delivery", "route", "fleet"],
    ))));
    registry
}

#[test]
fn a_task_is_routed_to_the_brain_whose_domain_it_names() {
    let registry = registry();

    let selected = registry
        .select(&AgentTask::new(
            "Optimize tomorrow's delivery route for the São Paulo fleet",
        ))
        .expect("a logistics task matches the router brain");

    assert_eq!(selected.brain.id(), "router");
    assert!(selected.match_result.score > 0.0);
}

#[test]
fn a_task_outside_every_domain_selects_no_brain() {
    assert!(registry()
        .select(&AgentTask::new(
            "Refactor the authentication module in Rust"
        ))
        .is_none());
}

#[tokio::test]
async fn the_json_interface_produces_the_same_answer_as_the_typed_call() {
    let typed = brain("counter", &["count"]);
    let erased = JsonBrain::new(typed.clone());
    let input = CountingInput {
        items: vec!["a".into(), "b".into(), "c".into()],
    };

    let direct = typed
        .solve(input.clone(), BrainSolveContext::default())
        .await
        .expect("typed solve");
    let through_json = erased
        .run(
            BrainOperation::Solve,
            serde_json::to_value(&input).unwrap(),
            BrainSolveContext::default(),
        )
        .await
        .expect("json solve");

    assert_eq!(through_json["total"], direct.total);
}

#[tokio::test]
async fn an_invalid_input_is_reported_through_the_json_interface() {
    let erased = JsonBrain::new(brain("counter", &["count"]));

    let report = erased
        .run(
            BrainOperation::Validate,
            serde_json::json!({ "items": [] }),
            BrainSolveContext::default(),
        )
        .await
        .expect("validation runs");

    assert_eq!(report["valid"], false);
    assert!(report["issues"][0]["message"]
        .as_str()
        .expect("issue message")
        .contains("at least one item"));
}

#[tokio::test]
async fn a_malformed_json_input_names_the_brain_that_rejected_it() {
    let erased = JsonBrain::new(brain("counter", &["count"]));

    let error = erased
        .run(
            BrainOperation::Solve,
            serde_json::json!({ "items": "not-a-list" }),
            BrainSolveContext::default(),
        )
        .await
        .map(|_| ())
        .expect_err("a malformed payload is rejected");

    assert!(error.to_string().contains("counter"), "{error}");
}

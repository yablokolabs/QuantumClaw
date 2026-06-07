use quantumclaw_core::{Result, SolverKind};
use quantumclaw_planner::{Plan, PlanScore, PlanStep, PlannerRationale};
use quantumclaw_policy::{DeterministicPolicyEngine, RiskLevel};

#[tokio::main]
async fn main() -> Result<()> {
    let plan = Plan {
        id: "release-cutover".into(),
        backend: "manual-review".into(),
        backend_kind: SolverKind::Classical,
        steps: vec![
            PlanStep::new("run migration dry-run", "shell").with_risk(RiskLevel::Medium),
            PlanStep::new("switch production traffic", "external-api").with_risk(RiskLevel::High),
            PlanStep::new("delete workspace fallback before validation", "shell")
                .with_risk(RiskLevel::Critical),
        ],
        score: PlanScore {
            utility: 0.72,
            confidence: 0.60,
            cost_estimate: 0.30,
            risk: 0.95,
        },
        rationale: PlannerRationale::new(
            "Release cutover proposal with one deliberately unsafe rollback step.",
        ),
        metadata: Default::default(),
    };

    let policy = DeterministicPolicyEngine::default();
    let decision = policy.evaluate_plan(&plan).await?;
    let audit = policy.audit_proposed_plan(&plan, &decision);

    println!("allowed: {}", decision.allowed);
    println!("risk: {}", decision.risk_level);
    println!("requires confirmation: {}", decision.required_confirmation);
    println!("reason: {}", decision.reasons.join("; "));
    println!("audit events: {}", audit.events.len());

    Ok(())
}

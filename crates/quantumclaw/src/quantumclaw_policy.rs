pub use crate::quantumclaw_core::PolicyEngine;
use crate::quantumclaw_core::Result;
use crate::quantumclaw_planner::Plan;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permission {
    pub resource: String,
    pub action: String,
}

impl Permission {
    pub fn new(resource: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            resource: resource.into(),
            action: action.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl Display for RiskLevel {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskLevel::Low => write!(f, "low"),
            RiskLevel::Medium => write!(f, "medium"),
            RiskLevel::High => write!(f, "high"),
            RiskLevel::Critical => write!(f, "critical"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HumanConfirmationThreshold {
    pub require_confirmation_at_or_above: RiskLevel,
}

impl Default for HumanConfirmationThreshold {
    fn default() -> Self {
        Self {
            require_confirmation_at_or_above: RiskLevel::High,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub reasons: Vec<String>,
    pub required_confirmation: bool,
    pub risk_level: RiskLevel,
}

impl PolicyDecision {
    pub fn allow(
        reason: impl Into<String>,
        risk_level: RiskLevel,
        required_confirmation: bool,
    ) -> Self {
        Self {
            allowed: true,
            reasons: vec![reason.into()],
            required_confirmation,
            risk_level,
        }
    }

    pub fn deny(reason: impl Into<String>, risk_level: RiskLevel) -> Self {
        Self {
            allowed: false,
            reasons: vec![reason.into()],
            required_confirmation: false,
            risk_level,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_type: String,
    pub plan_id: String,
    pub allowed: bool,
    pub risk_level: RiskLevel,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanAuditLog {
    pub events: Vec<AuditEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainPolicyPack {
    pub name: String,
    pub denied_tools: BTreeSet<String>,
    pub permissions: Vec<Permission>,
    pub threshold: HumanConfirmationThreshold,
}

#[derive(Debug, Clone, Default)]
pub struct DeterministicPolicyEngine {
    pub denied_tools: BTreeSet<String>,
    pub threshold: HumanConfirmationThreshold,
    pub domain_packs: Vec<DomainPolicyPack>,
}

impl DeterministicPolicyEngine {
    pub fn with_denied_tool(mut self, tool: impl Into<String>) -> Self {
        self.denied_tools.insert(tool.into());
        self
    }

    pub async fn evaluate_plan(&self, plan: &Plan) -> Result<PolicyDecision> {
        <Self as PolicyEngine>::evaluate_plan(self, plan).await
    }

    pub fn audit_proposed_plan(&self, plan: &Plan, decision: &PolicyDecision) -> PlanAuditLog {
        PlanAuditLog {
            events: vec![AuditEvent {
                event_type: "proposed_plan".into(),
                plan_id: plan.id.clone(),
                allowed: decision.allowed,
                risk_level: decision.risk_level,
                message: decision.reasons.join("; "),
            }],
        }
    }

    pub fn audit_executed_plan(&self, plan: &Plan, decision: &PolicyDecision) -> PlanAuditLog {
        PlanAuditLog {
            events: vec![AuditEvent {
                event_type: "executed_plan".into(),
                plan_id: plan.id.clone(),
                allowed: decision.allowed,
                risk_level: decision.risk_level,
                message: "execution audit recorded".into(),
            }],
        }
    }
}

#[async_trait]
impl PolicyEngine for DeterministicPolicyEngine {
    type Plan = Plan;
    type Decision = PolicyDecision;

    async fn evaluate_plan(&self, plan: &Self::Plan) -> Result<Self::Decision> {
        let risk = classify_plan(plan);
        for step in &plan.steps {
            if self.denied_tools.contains(&step.tool_name) {
                return Ok(PolicyDecision::deny(
                    format!(
                        "tool '{}' is denied by deterministic policy",
                        step.tool_name
                    ),
                    risk,
                ));
            }
        }

        if risk == RiskLevel::Critical {
            return Ok(PolicyDecision::deny(
                "critical risk plan rejected by deterministic policy",
                risk,
            ));
        }

        let requires_confirmation = risk >= self.threshold.require_confirmation_at_or_above;
        Ok(PolicyDecision::allow(
            "plan passed deterministic policy checks",
            risk,
            requires_confirmation,
        ))
    }
}

fn classify_plan(plan: &Plan) -> RiskLevel {
    let mut risk = RiskLevel::Low;
    for step in &plan.steps {
        let risk_text = step.risk_level.to_lowercase();
        let step_text =
            format!("{} {} {}", step.title, step.tool_name, step.rationale).to_lowercase();
        risk = risk.max(match risk_text.as_str() {
            "critical" => RiskLevel::Critical,
            "high" => RiskLevel::High,
            "medium" => RiskLevel::Medium,
            _ => RiskLevel::Low,
        });
        if step_text.contains("rm -rf")
            || step_text.contains("delete workspace")
            || step_text.contains("delete all")
        {
            risk = risk.max(RiskLevel::Critical);
        } else if step.tool_name == "shell" || step.tool_name == "code_edit" {
            risk = risk.max(RiskLevel::Medium);
        }
    }
    risk
}

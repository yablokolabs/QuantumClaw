use crate::quantumclaw_core::{Observer, Result, SolverKind};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    pub trace_id: String,
    pub event_type: String,
    pub message: String,
    pub selected_backend: Option<String>,
    pub rejected_backend: Option<String>,
    pub plan_score: Option<f64>,
    pub latency_ms: Option<u64>,
    pub cost_estimate: Option<f64>,
    pub confidence: Option<f64>,
    pub policy_decision: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

impl TraceEvent {
    pub fn new(event_type: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            trace_id: "trace-local".into(),
            event_type: event_type.into(),
            message: message.into(),
            selected_backend: None,
            rejected_backend: None,
            plan_score: None,
            latency_ms: None,
            cost_estimate: None,
            confidence: None,
            policy_decision: None,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricEvent {
    pub name: String,
    pub value: f64,
    pub dimensions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannerComparisonEvent {
    pub primary_backend: String,
    pub primary_backend_kind: SolverKind,
    pub shadow_backend: String,
    pub shadow_backend_kind: SolverKind,
    pub primary_score: f64,
    pub shadow_score: f64,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendTelemetry {
    pub backend: String,
    pub backend_kind: SolverKind,
    pub selected: bool,
    pub latency_ms: u64,
    pub cost_estimate: f64,
    pub confidence: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub events: Vec<TraceEvent>,
    pub metrics: Vec<MetricEvent>,
    pub comparisons: Vec<PlannerComparisonEvent>,
}

#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn write_audit(&self, event: Value) -> Result<()>;
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryObserver {
    trace_events: Arc<RwLock<Vec<TraceEvent>>>,
    metric_events: Arc<RwLock<Vec<MetricEvent>>>,
    comparison_events: Arc<RwLock<Vec<PlannerComparisonEvent>>>,
    audit_events: Arc<RwLock<Vec<Value>>>,
}

impl InMemoryObserver {
    pub fn record_trace(&self, event: TraceEvent) {
        self.trace_events
            .write()
            .expect("observer trace lock")
            .push(event);
    }

    pub fn record_metric(&self, event: MetricEvent) {
        self.metric_events
            .write()
            .expect("observer metric lock")
            .push(event);
    }

    pub fn record_comparison(&self, event: PlannerComparisonEvent) {
        self.comparison_events
            .write()
            .expect("observer comparison lock")
            .push(event);
    }

    pub fn execution_trace(&self) -> ExecutionTrace {
        ExecutionTrace {
            events: self
                .trace_events
                .read()
                .expect("observer trace lock")
                .clone(),
            metrics: self
                .metric_events
                .read()
                .expect("observer metric lock")
                .clone(),
            comparisons: self
                .comparison_events
                .read()
                .expect("observer comparison lock")
                .clone(),
        }
    }
}

#[async_trait]
impl Observer for InMemoryObserver {
    async fn observe(&self, event: Value) -> Result<()> {
        self.record_trace(TraceEvent::new("generic", event.to_string()));
        Ok(())
    }
}

#[async_trait]
impl AuditSink for InMemoryObserver {
    async fn write_audit(&self, event: Value) -> Result<()> {
        self.audit_events
            .write()
            .expect("observer audit lock")
            .push(event);
        Ok(())
    }
}

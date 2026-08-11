//! Q-Router as agent tools.
//!
//! These expose the brain through the runtime's existing tool contract, so an
//! agent can reach it the same way it reaches any other capability. The
//! implementation stays inside QuantumClaw; only the interface is tool-shaped.

use crate::benchmark::RouterBenchmark;
use crate::brain::{QRouterBrain, QRouterRequest};
use async_trait::async_trait;
use quantumclaw_brains::{BrainSolveContext, QuantumBrain};
use quantumclaw_core::{
    CoreToolCall, CoreToolResult, QuantumClawError, Result, SolverRegistry, Tool,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Tool name for optimizing a delivery plan.
pub const TOOL_OPTIMIZE: &str = "qrouter.optimize";
/// Tool name for comparing solvers on one problem.
pub const TOOL_BENCHMARK: &str = "qrouter.benchmark";
/// Tool name for checking an instance before optimizing it.
pub const TOOL_VALIDATE: &str = "qrouter.validate";
/// Tool name for listing and comparing available solvers.
pub const TOOL_COMPARE_SOLVERS: &str = "qrouter.compare_solvers";

/// Shared state behind every Q-Router tool.
#[derive(Clone, Default)]
pub struct QRouterToolContext {
    pub brain: Arc<QRouterBrain>,
    pub registry: Arc<SolverRegistry>,
}

impl QRouterToolContext {
    pub fn new(brain: Arc<QRouterBrain>, registry: Arc<SolverRegistry>) -> Self {
        Self { brain, registry }
    }

    fn solve_context(&self) -> BrainSolveContext {
        BrainSolveContext::default().with_registry(self.registry.clone())
    }
}

fn parse_request(call: &CoreToolCall) -> Result<QRouterRequest> {
    serde_json::from_value(call.input.clone()).map_err(|error| {
        QuantumClawError::new(format!(
            "{}: the input is not a valid routing request: {error}",
            call.tool_name
        ))
    })
}

fn ok(value: Value) -> CoreToolResult {
    CoreToolResult {
        success: true,
        output: value,
        metadata: BTreeMap::new(),
    }
}

fn encode<T: serde::Serialize>(name: &str, value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|error| {
        QuantumClawError::new(format!("{name}: could not encode the result: {error}"))
    })
}

/// Optimizes a delivery plan.
pub struct QRouterOptimizeTool {
    context: QRouterToolContext,
}

impl QRouterOptimizeTool {
    pub fn new(context: QRouterToolContext) -> Self {
        Self { context }
    }
}

#[async_trait]
impl Tool for QRouterOptimizeTool {
    fn name(&self) -> &str {
        TOOL_OPTIMIZE
    }

    fn description(&self) -> &str {
        "Optimize a delivery plan: assign deliveries to vehicles, sequence each route, and report logistics KPIs."
    }

    async fn call(&self, call: CoreToolCall) -> Result<CoreToolResult> {
        let request = parse_request(&call)?;
        let result = self
            .context
            .brain
            .solve(request, self.context.solve_context())
            .await?;
        Ok(ok(encode(TOOL_OPTIMIZE, result)?))
    }
}

/// Compares solvers on one problem.
pub struct QRouterBenchmarkTool {
    context: QRouterToolContext,
}

impl QRouterBenchmarkTool {
    pub fn new(context: QRouterToolContext) -> Self {
        Self { context }
    }
}

#[async_trait]
impl Tool for QRouterBenchmarkTool {
    fn name(&self) -> &str {
        TOOL_BENCHMARK
    }

    fn description(&self) -> &str {
        "Compare the customer's existing plan against classical and D-Wave backends on logistics KPIs."
    }

    async fn call(&self, call: CoreToolCall) -> Result<CoreToolResult> {
        let request = parse_request(&call)?;
        let backends: Vec<String> = call
            .input
            .get("backends")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_else(|| {
                let mut candidates = vec!["classical".to_string()];
                candidates.extend(self.context.registry.names());
                candidates
            });

        let report = RouterBenchmark::new((*self.context.brain).clone())
            .run(request, &backends, self.context.solve_context())
            .await?;
        Ok(ok(encode(TOOL_BENCHMARK, report)?))
    }
}

/// Validates an instance before anyone spends compute on it.
pub struct QRouterValidateTool {
    context: QRouterToolContext,
}

impl QRouterValidateTool {
    pub fn new(context: QRouterToolContext) -> Self {
        Self { context }
    }
}

#[async_trait]
impl Tool for QRouterValidateTool {
    fn name(&self) -> &str {
        TOOL_VALIDATE
    }

    fn description(&self) -> &str {
        "Check a routing instance for unreachable depots, impossible demands, and malformed matrices."
    }

    async fn call(&self, call: CoreToolCall) -> Result<CoreToolResult> {
        let request = parse_request(&call)?;
        let report = self.context.brain.validate(&request).await?;
        let valid = report.valid;
        Ok(CoreToolResult {
            success: valid,
            output: encode(TOOL_VALIDATE, report)?,
            metadata: BTreeMap::new(),
        })
    }
}

/// Reports which solvers are available and what the brain would pick.
pub struct QRouterCompareSolversTool {
    context: QRouterToolContext,
}

impl QRouterCompareSolversTool {
    pub fn new(context: QRouterToolContext) -> Self {
        Self { context }
    }
}

#[async_trait]
impl Tool for QRouterCompareSolversTool {
    fn name(&self) -> &str {
        TOOL_COMPARE_SOLVERS
    }

    fn description(&self) -> &str {
        "List registered solver backends and explain which one Q-Router would route this problem to."
    }

    async fn call(&self, call: CoreToolCall) -> Result<CoreToolResult> {
        let available = self.context.registry.names();
        let backends: Vec<Value> = self
            .context
            .registry
            .backends()
            .into_iter()
            .map(|backend| {
                let capabilities = backend.capabilities();
                json!({
                    "name": backend.name(),
                    "kind": backend.kind(),
                    "remote": capabilities.remote,
                    "requires_credentials": capabilities.requires_credentials,
                    "max_variables": capabilities.max_variables,
                    "supports_quadratic_models": capabilities.supports_quadratic_models,
                })
            })
            .collect();

        // When given a problem, also report the routing decision it would get.
        let routing = match parse_request(&call) {
            Ok(request) => {
                let formulations = self.context.brain.formulate(&request).await?;
                formulations
                    .iter()
                    .map(|formulation| {
                        let decision = self.context.brain.routing_policy.choose(
                            &formulation.class,
                            formulation.problem.variables.len(),
                            &available,
                        );
                        json!({
                            "subproblem": formulation.id,
                            "class": formulation.class,
                            "variables": formulation.problem.variables.len(),
                            "backend": decision.backend,
                            "reason": decision.reason,
                        })
                    })
                    .collect()
            }
            Err(_) => Vec::new(),
        };

        Ok(ok(json!({ "backends": backends, "routing": routing })))
    }
}

/// Builds every Q-Router tool, ready to register with a tool registry.
pub fn tools(context: QRouterToolContext) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(QRouterOptimizeTool::new(context.clone())),
        Arc::new(QRouterBenchmarkTool::new(context.clone())),
        Arc::new(QRouterValidateTool::new(context.clone())),
        Arc::new(QRouterCompareSolversTool::new(context)),
    ]
}

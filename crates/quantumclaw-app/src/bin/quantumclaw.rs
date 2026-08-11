//! The `quantumclaw` command line.
//!
//! One binary, one backend-selection mechanism. D-Wave backends are ordinary
//! names in the solver registry, so `--backend dwave-sa` needs no separate
//! command and no separate tool.

use quantumclaw_app::{brain_registry, solver_registry};
use quantumclaw_brains::QuantumBrain;
use quantumclaw_brains::{BrainOperation, BrainSolveContext};
use quantumclaw_brains_router::benchmark::RouterBenchmark;
use quantumclaw_brains_router::brain::{QRouterBrain, QRouterRequest};
use quantumclaw_core::{AgentTask, QuantumClawError, Result, SolverRegistry};
use quantumclaw_ir::DecisionProblem;
use quantumclaw_planner::{BackendSelectionPolicy, HybridPlanner, PlannerMode, PlannerRequest};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

const USAGE: &str = r#"quantumclaw - hybrid classical/quantum decision runtime

USAGE:
    quantumclaw <COMMAND> [OPTIONS]

COMMANDS:
    solve <problem.json> [--backend NAME]
        Solve a decision problem with one backend.

    benchmark <problem.json> --primary NAME --shadow NAME
        Run a primary and a shadow backend on the same problem and compare
        objective, feasibility, runtime, and optimality gap.

    backends
        List every registered solver backend and what it needs.

    brains
        List every registered domain brain.

    route <task text>
        Show which domain brain would handle an agent task.

    qrouter optimize <delivery.json> [--backend NAME] [--decomposition NAME]
    qrouter benchmark <delivery.json> [--backends a,b,c]
    qrouter validate <delivery.json>
    qrouter explain <delivery.json> [--backend NAME]
        Run the Q-Router logistics brain.

OPTIONS:
    --backend NAME     Solver backend, for example greedy-classical, dwave-sa,
                       dwave-exact, dwave-hybrid, dwave-qpu.
    --pretty           Pretty-print JSON output (default).
    --compact          Emit one-line JSON, for piping into other tools.

ENVIRONMENT:
    QUANTUMCLAW_DWAVE_PYTHON   Interpreter that runs the Ocean bridge.
    DWAVE_API_TOKEN            Read by Ocean inside the bridge for Leap/QPU.
"#;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("quantumclaw: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = arguments.first().map(String::as_str) else {
        println!("{USAGE}");
        return Ok(());
    };

    match command {
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        "solve" => solve(&arguments[1..]).await,
        "benchmark" => benchmark(&arguments[1..]).await,
        "backends" => backends(),
        "brains" => brains(),
        "route" => route(&arguments[1..]),
        "qrouter" => qrouter(&arguments[1..]).await,
        unknown => Err(QuantumClawError::new(format!(
            "unknown command '{unknown}'\n\n{USAGE}"
        ))),
    }
}

async fn solve(arguments: &[String]) -> Result<()> {
    let options = Options::parse(arguments)?;
    let path = options.require_positional("solve needs a problem file")?;
    let problem: DecisionProblem = read_json(&path)?;
    let registry = solver_registry();

    let backend_name = options
        .value("--backend")
        .unwrap_or_else(|| "greedy-classical".to_string());
    let backend = registry.require(&backend_name)?;

    let planner = HybridPlanner::default();
    let response = planner
        .plan(
            PlannerRequest::new(AgentTask::new(format!(
                "solve decision problem {}",
                problem.id
            )))
            .with_problem(problem)
            .with_selection_policy(BackendSelectionPolicy::prefer_backend(&backend_name))
            .with_backend(backend),
        )
        .await?;

    let plan = response.primary_plan();
    emit(
        &options,
        &SolveOutput {
            backend: plan.backend.clone(),
            backend_kind: format!("{:?}", plan.backend_kind),
            steps: plan.steps.iter().map(|step| step.title.clone()).collect(),
            score: plan.score.clone(),
            telemetry: response.telemetry.clone(),
        },
    )
}

async fn benchmark(arguments: &[String]) -> Result<()> {
    let options = Options::parse(arguments)?;
    let path = options.require_positional("benchmark needs a problem file")?;
    let problem: DecisionProblem = read_json(&path)?;
    let registry = solver_registry();

    let primary_name = options
        .value("--primary")
        .unwrap_or_else(|| "greedy-classical".to_string());
    let shadow_name = options
        .value("--shadow")
        .ok_or_else(|| QuantumClawError::new("benchmark needs --shadow NAME"))?;

    let response = HybridPlanner::default()
        .plan(
            PlannerRequest::new(AgentTask::new(format!("benchmark problem {}", problem.id)))
                .with_problem(problem)
                .with_mode(PlannerMode::ShadowCompare)
                .with_selection_policy(
                    BackendSelectionPolicy::prefer_backend(&primary_name).with_shadow(true),
                )
                .with_backend(registry.require(&primary_name)?)
                .with_shadow_backend(registry.require(&shadow_name)?),
        )
        .await?;

    emit(&options, &response.telemetry)
}

fn backends() -> Result<()> {
    let registry = solver_registry();
    for backend in registry.backends() {
        let capabilities = backend.capabilities();
        let mut notes = Vec::new();
        if capabilities.remote {
            notes.push("remote".to_string());
        }
        if capabilities.requires_credentials {
            notes.push("needs credentials".to_string());
        }
        if let Some(limit) = capabilities.max_variables {
            notes.push(format!("max {limit} variables"));
        }
        println!(
            "{:<28} {:<18} {}",
            backend.name(),
            format!("{:?}", backend.kind()),
            notes.join(", ")
        );
    }
    Ok(())
}

fn brains() -> Result<()> {
    for id in brain_registry().ids() {
        println!("{id}");
    }
    Ok(())
}

fn route(arguments: &[String]) -> Result<()> {
    let description = arguments.join(" ");
    if description.trim().is_empty() {
        return Err(QuantumClawError::new("route needs a task description"));
    }

    match brain_registry().select(&AgentTask::new(description)) {
        Some(selection) => {
            println!("brain: {}", selection.brain.id());
            println!("score: {:.2}", selection.match_result.score);
            println!("reason: {}", selection.match_result.reason);
        }
        None => println!("no domain brain matches this task; it stays with the general planner"),
    }
    Ok(())
}

async fn qrouter(arguments: &[String]) -> Result<()> {
    let Some(operation) = arguments.first().map(String::as_str) else {
        return Err(QuantumClawError::new(
            "qrouter needs an operation: optimize, benchmark, validate, or explain",
        ));
    };
    let options = Options::parse(&arguments[1..])?;
    let path = options.require_positional("qrouter needs a delivery problem file")?;
    let mut request: QRouterRequest = read_json(&path)?;
    if let Some(backend) = options.value("--backend") {
        request.options.backend = Some(backend);
    }
    if let Some(strategy) = options.value("--decomposition") {
        request.options.decomposition = Some(strategy);
    }

    let registry = Arc::new(solver_registry());
    let brain = QRouterBrain::new();
    let context = BrainSolveContext::default().with_registry(registry.clone());

    match operation {
        "optimize" => emit(&options, &brain.solve(request, context).await?),
        "validate" => emit(&options, &brain.validate(&request).await?.finish()),
        "explain" => {
            let result = brain.solve(request, context).await?;
            emit(&options, &brain.explain(&result).await?)
        }
        "benchmark" => {
            let backends: Vec<String> = match options.value("--backends") {
                Some(list) => list
                    .split(',')
                    .map(|name| name.trim().to_string())
                    .collect(),
                None => {
                    let mut candidates = vec!["classical".to_string()];
                    candidates.extend(registry.names());
                    candidates
                }
            };
            let report = RouterBenchmark::new(brain)
                .run(request, &backends, context)
                .await?;
            emit(&options, &report)
        }
        "formulate" => emit(&options, &brain.formulate(&request).await?),
        "decompose" => emit(&options, &brain.decompose(&request).await?),
        unknown => Err(QuantumClawError::new(format!(
            "unknown qrouter operation '{unknown}'; expected one of: {}",
            [
                BrainOperation::Validate,
                BrainOperation::Formulate,
                BrainOperation::Decompose,
                BrainOperation::Solve,
                BrainOperation::Explain,
            ]
            .iter()
            .map(|operation| format!("{operation:?}").to_lowercase())
            .collect::<Vec<_>>()
            .join(", ")
        ))),
    }
}

#[derive(Debug, Serialize)]
struct SolveOutput {
    backend: String,
    backend_kind: String,
    steps: Vec<String>,
    score: quantumclaw_planner::PlanScore,
    telemetry: quantumclaw_planner::PlannerTelemetry,
}

/// Minimal flag parser, matching the style already used by the assistant binary.
#[derive(Debug, Default)]
struct Options {
    positionals: Vec<String>,
    flags: BTreeMap<String, String>,
    compact: bool,
}

impl Options {
    fn parse(arguments: &[String]) -> Result<Self> {
        let mut options = Self::default();
        let mut index = 0;
        while index < arguments.len() {
            let argument = &arguments[index];
            match argument.as_str() {
                "--compact" => options.compact = true,
                "--pretty" => options.compact = false,
                flag if flag.starts_with("--") => {
                    let value = arguments
                        .get(index + 1)
                        .ok_or_else(|| QuantumClawError::new(format!("{flag} needs a value")))?;
                    options.flags.insert(flag.to_string(), value.clone());
                    index += 1;
                }
                positional => options.positionals.push(positional.to_string()),
            }
            index += 1;
        }
        Ok(options)
    }

    fn value(&self, flag: &str) -> Option<String> {
        self.flags.get(flag).cloned()
    }

    fn require_positional(&self, message: &str) -> Result<PathBuf> {
        self.positionals
            .first()
            .map(PathBuf::from)
            .ok_or_else(|| QuantumClawError::new(message.to_string()))
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<T> {
    let body = std::fs::read_to_string(path).map_err(|error| {
        QuantumClawError::new(format!("could not read {}: {error}", path.display()))
    })?;
    serde_json::from_str(&body).map_err(|error| {
        QuantumClawError::new(format!("{} is not a valid input: {error}", path.display()))
    })
}

fn emit<T: Serialize>(options: &Options, value: &T) -> Result<()> {
    let rendered = if options.compact {
        serde_json::to_string(value)
    } else {
        serde_json::to_string_pretty(value)
    }
    .map_err(|error| QuantumClawError::new(format!("could not render the result: {error}")))?;
    println!("{rendered}");
    Ok(())
}

/// Solver registries are cheap to build, so nothing here caches one.
#[allow(dead_code)]
fn unused(_: SolverRegistry) {}

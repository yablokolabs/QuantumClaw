use anyhow::{bail, Context, Result};
use quantumclaw_assistant::{
    quantum_plan_context, AssistantConfig, AssistantRunner, SarvamProvider, WorkspaceTools,
};
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug)]
struct Cli {
    workspace: PathBuf,
    task_file: PathBuf,
    report: Option<PathBuf>,
    max_turns: usize,
    max_total_tokens: u64,
}

#[derive(Debug, Serialize)]
struct RunReport {
    planner: quantumclaw_assistant::QuantumPlanContext,
    assistant: quantumclaw_assistant::AssistantReport,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("QuantumClaw assistant failed: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = parse_args(std::env::args().skip(1))?;
    let task = std::fs::read_to_string(&cli.task_file)
        .with_context(|| format!("failed to read task file {}", cli.task_file.display()))?;
    let planner = quantum_plan_context(&task).await?;
    let plan_context = serde_json::to_string_pretty(&planner)?;

    let provider = SarvamProvider::from_env()?;
    let tools = WorkspaceTools::new(&cli.workspace)?;
    let runner = AssistantRunner::new(
        provider,
        tools,
        AssistantConfig {
            max_turns: cli.max_turns,
            max_total_tokens: cli.max_total_tokens,
        },
    );
    let mut event_file = cli
        .report
        .as_ref()
        .map(|report| {
            let path = PathBuf::from(format!("{}.events.jsonl", report.display()));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(path)
        })
        .transpose()?;
    let mut event_error = None;
    let assistant_result = runner
        .run_observed(&task, &plan_context, |event| {
            if event_error.is_some() {
                return;
            }
            let result = (|| -> Result<()> {
                if let Some(file) = event_file.as_mut() {
                    serde_json::to_writer(&mut *file, event)?;
                    file.write_all(b"\n")?;
                    file.flush()?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                event_error = Some(error);
            }
        })
        .await;
    if let Some(error) = event_error {
        return Err(error).context("failed to persist assistant event log");
    }
    let assistant = assistant_result?;
    let report = RunReport { planner, assistant };
    let output = serde_json::to_string_pretty(&report)?;

    if let Some(path) = cli.report {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, format!("{output}\n"))?;
        println!("QuantumClaw report written to {}", path.display());
    } else {
        println!("{output}");
    }
    Ok(())
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Cli> {
    let mut workspace = None;
    let mut task_file = None;
    let mut report = None;
    let mut max_turns = 40usize;
    let mut max_total_tokens = 180_000u64;
    let mut args = args.into_iter();

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--workspace" => workspace = Some(PathBuf::from(next_value(&mut args, "--workspace")?)),
            "--task-file" => task_file = Some(PathBuf::from(next_value(&mut args, "--task-file")?)),
            "--report" => report = Some(PathBuf::from(next_value(&mut args, "--report")?)),
            "--max-turns" => {
                max_turns = next_value(&mut args, "--max-turns")?
                    .parse()
                    .context("--max-turns must be a positive integer")?;
                if max_turns == 0 {
                    bail!("--max-turns must be greater than zero");
                }
            }
            "--max-total-tokens" => {
                max_total_tokens = next_value(&mut args, "--max-total-tokens")?
                    .parse()
                    .context("--max-total-tokens must be a positive integer")?;
                if max_total_tokens == 0 {
                    bail!("--max-total-tokens must be greater than zero");
                }
            }
            "-h" | "--help" => {
                println!(
                    "quantumclaw-assistant \\\n  --workspace PATH \\\n  --task-file PATH \\\n  [--report PATH] \\\n  [--max-turns 40] \\\n  [--max-total-tokens 180000]\n\nSARVAM_API_KEY must be present in the process environment."
                );
                std::process::exit(0);
            }
            unknown => bail!("unknown argument: {unknown}"),
        }
    }

    Ok(Cli {
        workspace: workspace.context("--workspace is required")?,
        task_file: task_file.context("--task-file is required")?,
        report,
        max_turns,
        max_total_tokens,
    })
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next()
        .with_context(|| format!("{flag} requires a value"))
}

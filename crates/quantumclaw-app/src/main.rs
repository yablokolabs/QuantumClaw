#[tokio::main]
async fn main() {
    match quantumclaw_app::run_refactor_demo().await {
        Ok(report) => {
            println!("QuantumClaw demo completed");
            println!("task: {}", report.session.user_task);
            println!("backend: {}", report.plan.backend);
            println!("zeroclaw_runtime: {}", report.zeroclaw.runtime_name);
            println!(
                "zeroclaw_tool_substrate: {}",
                report.zeroclaw.tool_substrate
            );
            println!("zeroclaw_memory: {}", report.zeroclaw.memory_backend);
            println!("steps: {}", report.plan.steps.len());
            println!("learned_skill: {}", report.learned_skill.id);
        }
        Err(error) => {
            eprintln!("QuantumClaw demo failed: {error}");
            std::process::exit(1);
        }
    }
}

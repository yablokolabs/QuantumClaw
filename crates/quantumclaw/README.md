# quantumclaw

Single public crate for the QuantumClaw Rust workspace.

QuantumClaw is a ZeroClaw-backed agent runtime with backend-neutral planning traits, memory, policy, tools, skills, observability, and runtime orchestration.

## Usage

```toml
[dependencies]
quantumclaw = "0.1.0"
```

```rust
use quantumclaw::{AgentTask, SolverContext};

let task = AgentTask::new("Plan a safe coding refactor");
let context = SolverContext::from_task(&task);
```

## Public API

`quantumclaw` is the only crates.io-publishable package. Internal workspace crates are private (`publish = false`) and their public APIs are re-exported from this crate root.

Re-exported component APIs:

- `quantumclaw_core`
- `quantumclaw_runtime`
- `quantumclaw_memory`
- `quantumclaw_planner`
- `quantumclaw_ir`
- `quantumclaw_tools`
- `quantumclaw_policy`
- `quantumclaw_skills`
- `quantumclaw_observability`

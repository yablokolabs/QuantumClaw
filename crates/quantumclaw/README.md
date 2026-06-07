# quantumclaw

Umbrella crate for the QuantumClaw Rust workspace.

QuantumClaw is a ZeroClaw-backed agent runtime with backend-neutral planning traits, classical solvers, quantum-inspired solver scaffolds, and optional future QPU adapter boundaries.

## Usage

```toml
[dependencies]
quantumclaw = "0.1.0"
```

```rust
use quantumclaw::prelude::*;

let task = core::AgentTask::new("Plan a safe coding refactor");
let context = core::SolverContext::from_task(&task);
```

## Re-exported modules

- `core` — shared traits and runtime types
- `ir` — backend-neutral decision IR
- `planner` — planner modes and requests
- `memory` — memory abstractions
- `runtime` — ZeroClaw-backed runtime orchestration
- `tools` — tool registry and call abstractions
- `policy` — deterministic policy and audit controls
- `skills` — procedural skills and recipes
- `observability` — traces, metrics, and telemetry
- `solvers_classical` — classical solver backends
- `solvers_qinspired` — quantum-inspired solver scaffolds
- `solvers_future_qpu` — future QPU adapter scaffolds

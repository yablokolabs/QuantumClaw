# quantumclaw

Single public crate for the QuantumClaw Rust workspace.

QuantumClaw is a ZeroClaw-backed agent runtime with backend-neutral planning traits, memory, policy, tools, skills, observability, solver backends, and runtime orchestration.

## Install

End users only need one dependency:

```toml
[dependencies]
quantumclaw = "0.1.0"
```

Do not depend on `quantumclaw-core`, `quantumclaw-runtime`, `quantumclaw-planner`, or any other internal workspace crate. They are repo-internal implementation crates and are hidden from crates.io with `publish = false`.

## Usage

Prefer the prelude for application code:

```rust
use quantumclaw::prelude::*;

let task = AgentTask::new("Plan a safe coding refactor");
let context = SolverContext::from_task(&task);
```

Use short module paths when you want namespacing:

```rust
let planner = quantumclaw::planner::HybridPlanner::default();
let memory = quantumclaw::memory::InMemoryProceduralMemory::default();
let tools = quantumclaw::tools::InMemoryToolRegistry::with_default_tools();
let policy = quantumclaw::policy::DeterministicPolicyEngine::default();
```

## Public API

`quantumclaw` is the only crates.io-publishable package. Internal workspace crates stay private (`publish = false`) and their public APIs are mirrored into this crate.

Public module paths:

- `quantumclaw::prelude` — common application imports
- `quantumclaw::core` — shared traits and runtime primitives
- `quantumclaw::ir` — backend-neutral decision IR
- `quantumclaw::planner` — planner modes, requests, responses, and hybrid planning
- `quantumclaw::runtime` — ZeroClaw-backed runtime orchestration
- `quantumclaw::memory` — memory abstractions and in-memory stores
- `quantumclaw::tools` — tool registry and call abstractions
- `quantumclaw::policy` — deterministic policy and audit controls
- `quantumclaw::skills` — procedural skills and recipes
- `quantumclaw::observability` — traces, metrics, telemetry, and audit sinks
- `quantumclaw::solvers` — classical, quantum-inspired, and future-QPU solver APIs

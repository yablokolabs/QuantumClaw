# QuantumClaw spikes

Disposable feasibility experiments for solver/backend ideas before they become
production crates.

## Active spikes

- [`001-cudaq-qaoa-backend`](001-cudaq-qaoa-backend/): evaluates CUDA-Q as an
  optional QAOA-style solver sidecar behind QuantumClaw's `SolverBackend` trait.
  Verdict: PARTIAL until run in a CUDA-Q-ready environment.

# 001: CUDA-Q QAOA backend spike

## Question

Given QuantumClaw's backend-neutral `DecisionProblem` and current
`QuboLikeProblem` / `IsingLikeMapping` scaffold, when we serialize a small
planning problem into a QUBO-like JSON payload, then can a CUDA-Q sidecar run a
QAOA-style optimization and return a solver result that QuantumClaw can compare
against classical and quantum-inspired planners?

## Why this matters

QuantumClaw now uses ZeroClaw as the base runtime substrate. CUDA-Q should not
replace that. The useful question is narrower: can CUDA-Q become an optional
solver backend behind `SolverBackend`, giving QuantumClaw a real hybrid quantum
lane while the runtime, tools, memory, policy, observability, and planner API
stay stable?

CUDA-Q is a reasonable candidate because NVIDIA documents it as a Python/C++
programming model for hybrid CPU/GPU/QPU quantum programs, with CPU simulators,
NVIDIA GPU simulators, multi-GPU modes, and hardware provider backends.

## Prototype files

- `sample_problem.json`: minimal QUBO-like planning problem using the same shape
  as the current Rust scaffold.
- `cudaq_qaoa_spike.py`: Python sidecar that:
  - validates the JSON contract,
  - maps max-QUBO coefficients into an Ising Hamiltonian for minimizing the
    negative objective,
  - attempts a CUDA-Q QAOA path when `cudaq` is installed,
  - falls back to bounded classical brute force / greedy solving when CUDA-Q is
    unavailable so CI and normal dev hosts can still exercise the contract.

## Run

From the repository root:

```sh
# Syntax-only check. This does not import or execute CUDA-Q.
python3 -m py_compile spikes/001-cudaq-qaoa-backend/cudaq_qaoa_spike.py

# Explicit CUDA-Q import probe. This is the command that proves whether
# `import cudaq` works on the current host.
python3 spikes/001-cudaq-qaoa-backend/cudaq_qaoa_spike.py --check-cudaq-import

# Run the sidecar. It uses CUDA-Q only when the import probe succeeds; otherwise
# it returns a classical fallback result to keep the contract runnable in CI.
python3 spikes/001-cudaq-qaoa-backend/cudaq_qaoa_spike.py

# Force real CUDA-Q mode and fail instead of falling back.
python3 spikes/001-cudaq-qaoa-backend/cudaq_qaoa_spike.py --require-cudaq
```

Current explicit CUDA-Q import probe on this VM:

```json
{
  "cudaq_available": false,
  "reason": "CUDA-Q unavailable: No module named 'cudaq'"
}
```

Current fallback run on this VM:

```json
{
  "best_bitstring": "1111",
  "best_score": 3.17,
  "candidate_count": 16,
  "cudaq_available": false,
  "mode": "classical-fallback",
  "reason": "CUDA-Q unavailable: No module named 'cudaq'",
  "selected_actions": [
    "inspect",
    "test-first",
    "small-edit",
    "validate"
  ],
  "solver": "classical-bruteforce",
  "variables": [
    "inspect",
    "test-first",
    "small-edit",
    "validate"
  ]
}
```

Environment finding: this VM currently has no visible NVIDIA GPU via
`nvidia-smi`, and Python `cudaq` is not installed. That means this spike validates
the sidecar contract locally, not CUDA-Q execution or CUDA-Q performance.

## Integration shape if validated

1. Add optional crate/feature: `quantumclaw-solvers-cudaq` or
   `quantumclaw-solvers-future-qpu --features cudaq`.
2. Keep ZeroClaw as the runtime substrate.
3. Keep QuantumClaw's Rust `SolverBackend` trait as the boundary.
4. Encode `DecisionProblem` into `QuboLikeProblem` JSON.
5. Spawn a Python CUDA-Q sidecar or a long-lived local sidecar process.
6. Decode returned bitstrings / scores into `SolverOutput`.
7. Use `PlannerMode::ShadowCompare` to benchmark CUDA-Q against
   `GreedySolver` and `QuantumInspiredSolver` before making it selectable by
   default.

## Risks and unknowns

- CUDA-Q is Python/C++ first; there is no obvious native Rust crate to depend on.
- Setup can be heavy and backend-specific.
- QAOA circuit construction needs problem-specific tuning; a direct QUBO mapping
  may not beat classical heuristics for small planning graphs.
- GPU/QPU acceleration only matters for problem sizes and structures where the
  quantum path has a measurable advantage.
- Deterministic policy still has to gate the decoded plan; solver output must not
  bypass QuantumClaw policy.

## Verdict: PARTIAL

### What worked

- A concrete sidecar contract exists and runs from the repo.
- The sample QUBO-like planning payload is validated.
- The fallback path returns a solver-shaped result suitable for adapter testing.
- The CUDA-Q path is isolated behind optional runtime import instead of becoming
  a hard dependency.

### What did not work locally

- CUDA-Q itself was not executed because `cudaq` is not installed on this VM.
- No NVIDIA GPU is visible locally.

### Recommendation for the real build

Do not add CUDA-Q as a core dependency yet. Keep it as an optional spike/backend
lane. Next validation should run this sidecar inside a CUDA-Q-ready environment
using `qpp-cpu` first, then `nvidia` GPU target, and compare latency/quality under
`ShadowCompare` before promoting it into a production Rust crate.

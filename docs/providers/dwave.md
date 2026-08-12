# D-Wave Ocean provider

QuantumClaw treats D-Wave as one interchangeable optimization provider among
others. Selecting it is a configuration choice — a backend name — not an
architectural commitment, and no D-Wave type appears anywhere in QuantumClaw's
domain models.

```
DecisionProblem / OptimizationProblem
        ↓
QUBO/BQM compiler  (quantumclaw-optimization)
        ↓
SolverBackend      (quantumclaw-core)
        ├── greedy-classical, beam-search-classical, …
        ├── quantum-inspired-hybrid
        ├── dwave-sa       → classical simulated annealing (local)
        ├── dwave-exact    → classical exhaustive search (local)
        ├── dwave-hybrid   → D-Wave Leap hybrid solver (cloud)
        └── dwave-qpu      → quantum annealing hardware (cloud)
        ↓
SolverOutput + OptimizationSolution + provider metadata
```

## The three execution lanes

### Local development

```
QuantumClaw → Ocean → dwave.samplers.SimulatedAnnealingSampler → classical simulated annealing
```

**`SimulatedAnnealingSampler` is a classical algorithm running on your CPU. It
is not a QPU simulator and it does not emulate quantum annealing.** It is a
metaheuristic that happens to operate on the same BQM representation D-Wave
hardware accepts, which makes it the right tool for developing and testing a
QUBO formulation before any quantum resource is involved. Nothing in this
repository claims otherwise, and results from this lane must never be described
as quantum results.

Requires no D-Wave account, no credentials, and no network access.

### Hybrid production option

```
QuantumClaw → Ocean → dwave.system.LeapHybridSampler → D-Wave managed quantum/classical optimization
```

Leap hybrid solvers combine classical compute with quantum hardware and accept
much larger problems than a bare QPU. Requires Leap credentials.

### Quantum annealing option

```
QuantumClaw → Ocean → DWaveSampler + EmbeddingComposite → minor embedding → D-Wave QPU
```

The problem graph must be embedded onto the physical qubit topology.
Not every problem can be embedded, and embedding failures surface as a typed
`embedding_failed` error rather than a silent fallback. Requires Leap
credentials.

## Installation

QuantumClaw's core is Rust; Ocean is Python. The two meet at a small JSON
sidecar, so Ocean's dependency tree never enters a QuantumClaw build.

```sh
# Local classical samplers only — no cloud client, no credentials.
pip install "quantumclaw-dwave[local]"

# Everything, including the Leap cloud client, QPU sampler, and embedding tools.
pip install "quantumclaw-dwave[dwave]"
```

When developing against a checkout, install the bridge from its path instead so
your edits take effect:

```sh
pip install -e "crates/quantumclaw-providers-dwave/python[local,test]"
```

Then point QuantumClaw at the interpreter that has it:

```sh
export QUANTUMCLAW_DWAVE_PYTHON=/path/to/venv/bin/python
```

If Ocean is missing, backends fail with

```
D-Wave Ocean backend is not installed. Install it with:
pip install 'quantumclaw-dwave[dwave]'
```

rather than a stack trace. Registration itself never touches Ocean, so
`quantumclaw backends` always lists the D-Wave lanes even on a machine that
cannot run them.

## Configuration

| Variable | Purpose |
| --- | --- |
| `QUANTUMCLAW_DWAVE_PYTHON` | Interpreter that runs the bridge (default `python3`) |
| `QUANTUMCLAW_DWAVE_BRIDGE` | Explicit bridge script path, instead of the installed module |
| `QUANTUMCLAW_DWAVE_PYTHONPATH` | Extra `PYTHONPATH` entries, for running from a checkout |
| `QUANTUMCLAW_DWAVE_TIMEOUT_MS` | Per-solve timeout; the child is killed when it expires |
| `DWAVE_API_TOKEN` | Leap API token — **read by Ocean inside the bridge, never by QuantumClaw** |
| `DWAVE_ENDPOINT` | Leap API endpoint |
| `DWAVE_SOLVER` | Solver name or selection filter |
| `DWAVE_REGION` | Leap region |
| `DWAVE_PROFILE` | Profile in `~/.config/dwave/dwave.conf` |

Sampling parameters are set in code:

```rust
use quantumclaw::providers::dwave::{DWaveSimulatedAnnealingBackend, SimulatedAnnealingParams};

let backend = DWaveSimulatedAnnealingBackend::from_env().with_params(
    SimulatedAnnealingParams::default()
        .with_num_reads(500)
        .with_num_sweeps(2_000)
        .with_beta_range(0.1, 4.2)
        .with_seed(42),
);
```

`dwave-exact` takes `ExactParams::with_max_variables` (default 20),
`dwave-hybrid` takes `HybridParams::with_time_limit_s`, and `dwave-qpu` takes
`QpuParams` for `num_reads`, `chain_strength`, and `annealing_time_us`.

## CLI

```sh
quantumclaw backends                                   # what is available, and what it needs
quantumclaw solve problem.json --backend dwave-sa
quantumclaw solve problem.json --backend dwave-hybrid
quantumclaw benchmark problem.json --primary greedy-classical --shadow dwave-sa
```

## Observability

Every run attaches structured provider metadata to `SolverOutput.telemetry`:

```json
{
  "provider": "dwave",
  "backend": "simulated_annealing",
  "sampler": "dwave.samplers.SimulatedAnnealingSampler",
  "problem_type": "BQM",
  "variables": 21,
  "interactions": 45,
  "num_reads": 100,
  "objective": 146.0,
  "energy": -3.5,
  "feasible": true,
  "violations": 0,
  "runtime_ms": 231,
  "solver_runtime_ms": 12.4
}
```

`qpu_access_time_us`, `chain_break_fraction`, `hybrid_run_time_us`, and
`charge_time_us` appear only when the backend that ran actually measured them.
A classical sampler never reports a QPU timing. Read it back typed with
`DWaveRunMetadata::from_telemetry(&output.telemetry)`.

Wall time (`runtime_ms`, measured by QuantumClaw and including interpreter
startup) and in-solver time (`solver_runtime_ms`, measured inside the bridge)
are reported separately, because one process launch per solve would otherwise
dominate any benchmark of a small problem.

## Errors

Provider failures are typed and never swallowed; the message Ocean produced is
preserved as the cause.

| Code | Meaning |
| --- | --- |
| `ocean_missing` | Ocean is not importable on the configured interpreter |
| `bridge_spawn_failed` | The sidecar process could not be started |
| `bridge_protocol_error` | The sidecar returned something that is not a response |
| `timeout` | The solve exceeded the configured timeout; the child was killed |
| `invalid_bqm` | The compiled model was rejected |
| `invalid_configuration` | Backend parameters are unusable |
| `missing_credentials` | No Leap token in the environment or `dwave.conf` |
| `authentication_failed` | Leap rejected the credentials |
| `solver_unavailable` | The requested solver is offline, unknown, or unreachable |
| `embedding_failed` | The problem could not be embedded onto the QPU topology |
| `problem_too_large` | Above the backend's declared variable limit |
| `no_feasible_result` | The sampler returned nothing usable |
| `sampler_failed` | Any other provider-side failure |
| `compilation_failed` | The problem could not be turned into a binary model |

## Security

- **The API token never enters QuantumClaw.** Ocean reads `DWAVE_API_TOKEN` or
  `~/.config/dwave/dwave.conf` inside the bridge process. QuantumClaw only
  checks whether a credential is *present*, so it can fail early with a useful
  message. The token is never logged, never serialized into telemetry, and
  never passed as a process argument where `ps` would expose it.
- `LeapConfig` carries only non-secret settings: solver, region, endpoint,
  profile.
- The bridge is a subprocess with piped stdio and a hard timeout; it is killed
  on drop.
- Problem data is sent to D-Wave when you select `dwave-hybrid` or `dwave-qpu`.
  Treat that as any other third-party data processor.

## Limitations

- One process per solve. Fine for the problem sizes that suit a QUBO; a
  long-lived sidecar would be the next step if throughput ever matters.
- Inequality constraints are encoded with binary slack variables and require
  integral coefficients. Scale your units first; a non-integral coefficient
  produces an `UnsupportedConstraint` error rather than a silently wrong model.
- `dwave-exact` enumerates `2^n` assignments. Its default 20-variable limit is
  a guard, not a suggestion.
- No benchmark in this repository shows a quantum advantage. On the bundled
  logistics example, `dwave-sa` and the classical path reach the same objective.
  That is the honest current state, and the ShadowCompare machinery exists
  precisely so the claim can be re-tested rather than assumed.

## Testing

```sh
# Rust: Ocean-dependent tests skip cleanly when the interpreter is not set.
cargo test --workspace
QUANTUMCLAW_DWAVE_PYTHON=/path/to/venv/bin/python \
  QUANTUMCLAW_DWAVE_REQUIRE=1 cargo test --workspace

# Python bridge
cd crates/quantumclaw-providers-dwave/python && pytest
```

`QUANTUMCLAW_DWAVE_REQUIRE=1` turns "skipped" into "failed", which is how CI
proves the Ocean lane actually ran. No test requires a Leap account, a QPU, or
any paid resource.

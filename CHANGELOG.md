# Changelog

All notable changes to the published `quantumclaw` crate.

This project follows [Semantic Versioning](https://semver.org/). While the crate
is below `1.0.0`, breaking changes bump the minor version.

## [Unreleased]

### Added

- Opt-in QUBO route sequencing (`SequencingPolicy`). Position-encoded TSP over
  `n^2` binaries, guarded by a size threshold, and applied only when the
  sampled tour is strictly shorter than the classical one and decodes to a
  valid permutation.
- `sequencing_benchmark` example, which scores both methods against the exact
  optimum by enumerating permutations.

### Notes

- Measured over 240 seeded instances: classical local search stays the better
  default. The QUBO lane matches or beats it at 5-6 stops with generous
  `num_reads`, and falls behind from 7 stops upward. See
  `docs/brains/qrouter.md`.

## [0.2.2] - 2026-08-12

### Added

- The MIT licence text now ships inside the published package. Every manifest
  already declared MIT, but the file itself was missing, so the crate on
  crates.io advertised a licence its consumers could not read.

No API or behaviour changes. Upgrading from 0.2.1 needs no code edits.

## [0.2.1] - 2026-08-12

### Changed

- The `ocean_missing` error and every install instruction now point at
  `pip install 'quantumclaw-dwave[dwave]'`. The bridge is published on PyPI as
  [`quantumclaw-dwave`](https://pypi.org/project/quantumclaw-dwave/), so the
  repository-URL install that 0.2.0 suggested is no longer necessary.

### Fixed

- Cleared five yanked transitive dependencies from `Cargo.lock`
  (`async-utility`, `nostr`, `nostr-relay-pool`, `bitcoin-io`,
  `bitcoin_hashes`). No manifest or API change; lockfiles do not affect
  consumers of a published library.

No API changes. Upgrading from 0.2.0 needs no code edits.

## [0.2.0] - 2026-08-12

### Added

- **Domain-neutral optimization layer.** `quantumclaw::ir::optimization` adds
  binary variables, linear and quadratic terms, reusable constraints
  (`ExactlyOne`, `AtMostOne`, `AtLeastOne`, `Implication`, `Conflict`,
  `LinearEquality`, `LinearAtMost`), a `BinaryQuadraticModel`, and a normalized
  `OptimizationSolution` carrying feasibility and per-constraint violations.
  `quantumclaw::optimization` compiles those into a minimization QUBO with
  penalty encoding, binary slack for inequalities, and auto-scaled penalty
  weights, then decodes samples back into objective values in the caller's own
  units.
- **D-Wave Ocean provider** (`quantumclaw::providers::dwave`) with four
  backends selectable by name: `dwave-sa` (classical simulated annealing),
  `dwave-exact` (classical exhaustive search), `dwave-hybrid` (Leap hybrid
  solver), and `dwave-qpu` (quantum annealing with minor embedding). Ocean is
  reached through a Python sidecar, so its dependency tree never enters a
  QuantumClaw build.
- **Quantum brain layer** (`quantumclaw::brains`): the `QuantumBrain` trait,
  an object-safe `ErasedBrain` with a `JsonBrain` adapter, and a
  `BrainRegistry` that routes agent tasks to a domain brain.
- **Q-Router** (`quantumclaw::brains::router`), the first domain brain:
  logistics modelling (heterogeneous fleets, depots, capacities, delivery
  windows, distance matrices, fuel, CO2, SLA), decomposition strategies for
  large instances, vehicle-assignment QUBO compilation, classical route
  sequencing, plan repair and re-validation, logistics KPI evaluation, and
  baseline-versus-solver benchmarking. Exposed as the `qrouter.optimize`,
  `qrouter.benchmark`, `qrouter.validate`, and `qrouter.compare_solvers` tools.
- `SolverRegistry` for name-keyed backend lookup, and `SolverCapabilities`
  reported through a defaulted `SolverBackend::capabilities` method.
- Name-based backend selection via `BackendSelectionPolicy::prefer_backend`,
  which fails loudly when the requested backend is unavailable rather than
  silently choosing another.
- `OptimizationComparison` on `ShadowComparison`: objective delta, relative
  gap, feasibility, violation counts, and separate solver/wall runtimes.
- A `quantumclaw` binary with `solve`, `benchmark`, `backends`, `brains`,
  `route`, and `qrouter` subcommands.

### Changed

- `BackendTelemetry` reports structured provider metadata. Fields a backend
  cannot measure stay absent rather than being filled with plausible numbers.
- Tool results now survive the ZeroClaw adapter round trip as structured JSON
  instead of being flattened into a string.

### Breaking

Downstream code that constructs these types with struct literals, or matches
exhaustively on `SolverKind`, needs updating:

- `SolverKind` gained `QuantumHybrid` and `QuantumAnnealing`.
- `SolverOutput` gained `solution: Option<OptimizationSolution>`.
- `BackendTelemetry` gained `provider` and `provider_metadata`.
- `DecisionProblem` gained `optimization: Option<OptimizationProblem>`.
- `ShadowComparison` gained `primary_latency_ms` and `optimization`.
- `BackendSelectionPolicy` gained `preferred_backend`.

All new fields are `Option` or defaulted, so `..Default::default()` and the
existing constructors keep working.

### Notes

- `dwave-sa` runs Ocean's `SimulatedAnnealingSampler` and reports
  `SolverKind::Classical`. It is classical simulated annealing over an
  Ocean-compatible BQM, **not** a QPU simulator.
- The `dwave-hybrid` and `dwave-qpu` backends have not been exercised against
  live D-Wave hardware or the Leap service. Their failure paths are tested;
  their success paths are unverified.
- The Python bridge was not on PyPI at the time of this release and installed
  from the repository. It is published as of 0.2.1.
- No benchmark in this release demonstrates a quantum advantage.

## [0.1.0] - 2026-06-07

Initial release: ZeroClaw-based agent runtime with classical and
quantum-inspired planners, backend-neutral decision IR, memory, tools, policy,
skills, and observability behind a single public crate.

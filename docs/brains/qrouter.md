# Q-Router: the logistics quantum brain

Q-Router is a domain brain inside QuantumClaw, not an application on top of it.
It owns logistics knowledge and nothing else: the generic optimization layer
below it stays domain-neutral, and the solver layer below that stays
provider-neutral.

```
agent task
    ↓
Quantum Brain Registry              (quantumclaw-brains)
    ↓
Q-Router brain                      (quantumclaw-brains-router)
    ↓  validate → decompose → formulate
OptimizationProblem                 (quantumclaw-ir::optimization)
    ↓  QUBO/BQM compilation
SolverBackend                       (classical | dwave-sa | dwave-hybrid | dwave-qpu)
    ↓
route reconstruction, repair, classical sequencing
    ↓
constraint re-check + logistics KPIs
    ↓
agent response
```

## What lives where

| Concern | Owner |
| --- | --- |
| VRP, CVRP, VRPTW, fleets, depots, windows, fuel, CO2, SLA | Q-Router |
| Binary variables, objectives, constraints, QUBO compilation | `quantumclaw-optimization` |
| Solver selection, execution, telemetry | `quantumclaw-core` + providers |
| D-Wave | provider layer only |

Q-Router does not depend on any provider crate, and a test enforces that. It
asks a `SolverRegistry` for a backend by name and treats every backend
identically.

## The QuantumBrain abstraction

```rust
#[async_trait]
pub trait QuantumBrain: Send + Sync {
    type Input;
    type Output;

    fn id(&self) -> &str;
    fn capabilities(&self) -> BrainCapabilities;
    fn can_handle(&self, task: &AgentTask) -> BrainMatch;

    async fn validate(&self, input: &Self::Input) -> Result<ValidationReport>;
    async fn plan(&self, input: &Self::Input) -> Result<BrainPlan>;
    async fn formulate(&self, input: &Self::Input) -> Result<Vec<Formulation>>;
    async fn decompose(&self, input: &Self::Input) -> Result<Decomposition>;
    async fn solve(&self, input: Self::Input, ctx: BrainSolveContext) -> Result<Self::Output>;
    async fn evaluate(&self, output: &Self::Output) -> Result<KpiReport>;
    async fn explain(&self, output: &Self::Output) -> Result<Explanation>;
}
```

`JsonBrain<B>` adapts any such brain into the object-safe `ErasedBrain`, which
is what the registry, the tools, and agents use. Future brains — Q-Scheduler,
Q-Portfolio, Q-ResourceAllocator — implement the same trait and register the
same way. Nothing about the registry is routing-specific.

## Scale: decomposition, not one giant QUBO

A 10,000-delivery instance is not a single QUBO, and Q-Router never pretends
otherwise. It partitions first and formulates second:

| Strategy | Splits by |
| --- | --- |
| `single-block` | nothing; for instances that already fit |
| `depot-partition` | depot, with unassigned stops going to the nearest one |
| `geographic-cluster` | a deterministic sweep around the depot, bounded cluster size |
| `capacity-cluster` | cumulative demand against a block of vehicles |
| `time-window-partition` | service time bucket |
| `rolling-horizon` | consecutive horizons across the day |

`DecompositionPolicy` escalates through these until every piece fits
`max_variables_per_subproblem`. Every strategy produces a partition: each
delivery appears in exactly one subproblem, which is asserted by test.

## Which parts go to a sampler

Only **vehicle assignment** is compiled to a QUBO. It is a genuine
combinatorial choice with quadratic structure — the interaction term prices two
stops carried by the same vehicle by the distance between them, which is what
makes clustering emerge from the objective rather than from a heuristic.

**Route sequencing is classical by default** (nearest neighbour, 2-opt,
or-opt). A permutation-encoded TSP QUBO lane exists and is opt-in — see
[Sequencing: measured, not assumed](#sequencing-measured-not-assumed) for the
numbers that justify the default.

`SolverRoutingPolicy` then decides per subproblem:

1. An explicitly requested backend always wins, and failing it is an error —
   never a silent downgrade to classical.
2. Otherwise, recorded evidence wins: `BenchmarkLedger` tracks objective,
   feasibility, and runtime per problem class and size bucket, and the best
   *feasible* mean objective is chosen. Infeasible runs never recommend a
   backend.
3. Otherwise the configured preference, if it is registered.
4. Otherwise the brain solves it classically.

The ledger is in-memory with `to_json`/`from_json`, so persistence is the
caller's decision.

## Sequencing: measured, not assumed

A TSP QUBO lane is available for route sequencing, off by default. It uses the
standard position encoding — `x[stop][position]`, `n²` binaries, one
`ExactlyOne` per stop and per position, distances as quadratic terms between
consecutive positions.

```rust
request.options.sequencing = SequencingPolicy::default()
    .enabled()
    .with_max_stops(6)
    .with_backend("dwave-sa");
```

Three safeguards apply. It is opt-in; a size guard refuses routes above
`max_stops` (default 8, because `n²` grows fast); and a sampled tour replaces
the classical one **only if it is strictly shorter and decodes to a valid
permutation**. Invalid samples — two stops in one position, a stop left out —
are rejected rather than turned into a route. Enabling the lane can cost
runtime; it cannot cost route quality.

### What it actually does

240 seeded instances, both methods scored against the exact optimum found by
enumerating every permutation. Reproduce with:

```sh
QUANTUMCLAW_DWAVE_PYTHON=… cargo run -p quantumclaw-app --release \
    --example sequencing_benchmark
```

At the default 100 reads:

| stops | vars | classical optimal | QUBO optimal | classical excess | QUBO excess | classical | QUBO |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 3 | 9 | 20/20 | 20/20 | 0.00% | 0.00% | 0.01 ms | 218 ms |
| 4 | 16 | 20/20 | 20/20 | 0.00% | 0.00% | 0.01 ms | 210 ms |
| 5 | 25 | 19/20 | 16/20 | 0.00% | 0.94% | 0.01 ms | 216 ms |
| 6 | 36 | 18/20 | 6/20 | 0.04% | 3.00% | 0.02 ms | 222 ms |
| 7 | 49 | 17/20 | 1/20 | 0.19% | 10.85% | 0.03 ms | 236 ms |
| 8 | 64 | 16/20 | 0/20 | 0.26% | 17.35% | 0.05 ms | 248 ms |

At 2000 reads the picture improves but does not reverse:

| stops | classical optimal | QUBO optimal | QUBO excess | head to head |
| --- | --- | --- | --- | --- |
| 5 | 19/20 | **20/20** | 0.00% | QUBO shorter once |
| 6 | 18/20 | **20/20** | 0.00% | QUBO shorter twice |
| 7 | 17/20 | 8/20 | 1.83% | classical shorter 12× |
| 8 | 16/20 | 4/20 | 3.89% | classical shorter 15× |

Reading these honestly:

- **The encoding is correct.** Zero invalid tours in 240 runs, and with enough
  reads the QUBO finds the exact optimum on every 5- and 6-stop instance,
  beating classical outright three times.
- **It degrades with size faster than effort compensates.** At 8 stops,
  20× the sampling effort moves it from 0/20 to 4/20.
- **Classical is not perfect either** (16/20 at 8 stops) but its misses cost
  0.26% while the QUBO's cost 3.89%.
- **The cost gap is four orders of magnitude**, and process startup is only
  part of it.

So: classical stays the default. The QUBO lane is competitive at 5–6 stops
with generous `num_reads`, and is worth enabling as a second opinion when
local search is suspected of sticking in a bad optimum — not as a routine
setting, and not because it is quantum. `dwave-sa` is classical simulated
annealing either way.

## Using it

```rust
use quantumclaw::brains::router::{QRouterBrain, QRouterRequest};
use quantumclaw::brains::{BrainSolveContext, QuantumBrain};

let result = QRouterBrain::new()
    .solve(
        QRouterRequest::new(delivery_problem).with_backend("dwave-sa"),
        BrainSolveContext::default().with_registry(registry),
    )
    .await?;
```

From the CLI:

```sh
quantumclaw route "Optimize tomorrow's deliveries from the São Paulo depot using 25 trucks"
quantumclaw qrouter validate  deliveries.json
quantumclaw qrouter decompose deliveries.json
quantumclaw qrouter optimize  deliveries.json --backend dwave-sa
quantumclaw qrouter benchmark deliveries.json --backends classical,dwave-sa
quantumclaw qrouter explain   deliveries.json
```

As agent tools: `qrouter.optimize`, `qrouter.benchmark`, `qrouter.validate`,
`qrouter.compare_solvers`, registered through the runtime's existing tool
registry.

## Benchmarking is the point

Q-Router compares the customer's own plan against every candidate solver using
one KPI implementation, so the numbers are comparable:

total distance · total travel time · vehicles used · fleet utilization ·
capacity utilization · deliveries served · unassigned · late deliveries ·
SLA violation minutes · SLA breaches · estimated fuel · estimated CO2 ·
estimated operating cost · objective value · feasibility · optimization runtime
· solver runtime

### Runs, not draws

Samplers are stochastic: one run is a draw from a distribution, not a
measurement. Every candidate is run `--repeat` times (default 5) with seeds
derived from `--seed`, and the report carries best, median, worst, mean and
standard deviation. The same seed reproduces the same report exactly.

Candidates are ranked on **how often they were feasible first, then on median
objective**. Ranking on the best run would reward luck; ignoring feasibility
would reward plans that strand deliveries. When the winner was not feasible on
every run, the report says so rather than presenting it as simply best.

Ask for the classical path by name — `classical`. Leaving the backend unset
means "let the routing policy decide", and the policy prefers a sampler, so an
unset backend is not a classical baseline.

Only a **feasible** plan can win a benchmark. A cheaper plan that strands a
delivery or overloads a truck is not a cheaper plan.

Example output on the bundled São Paulo instance:

```
baseline   objective=267.28                       feasible=false  (fixed plan)
classical  median=234.49  sd= 0.00  feasible=0/5
dwave-sa   median=226.39  sd=10.67  feasible=4/5
winner: dwave-sa
note: the winner 'dwave-sa' was feasible on only 4 of 5 runs;
      treat it as unreliable rather than best
```

Three real findings in one table. The customer's own plan is infeasible — it
overloads a truck. The classical heuristic is perfectly consistent
(`sd=0.00`) and never finds a servable plan on this instance, which is the
bin-packing limitation described below. And `dwave-sa` finds a better plan
most of the time but not every time, so the report refuses to call it simply
"best".

None of that is a quantum result: `dwave-sa` is classical simulated annealing.
What it shows is a QUBO formulation succeeding where a greedy heuristic gets
stuck, at the cost of consistency.

## Enterprise workflow this supports

```
customer historical routing data
    → baseline reproduction and KPI evaluation
    → classical optimization
    → Ocean simulated annealing benchmark
    → ShadowCompare
    → identify problem classes where hybrid/quantum methods look promising
    → optionally execute those workloads on Leap or a QPU
    → compare operational KPIs
```

No step of that assumes quantum wins. The ledger exists so the decision is
made from measurements.

## Limitations

- Assignment costs are a proxy (out-and-back travel plus pairwise proximity),
  not exact post-sequencing route length. The decoder re-sequences and re-checks
  everything, so a plan is always validated in real terms — but the QUBO
  optimum and the route optimum are not identical.
- The classical fallback is cheapest-insertion, which is fast and clusters well
  but cannot solve a tight bin-packing. On a fleet at 100% capacity it can
  strand a delivery; it reports that as an unassigned delivery and an
  infeasible plan rather than hiding it, and the compiled model finds the
  packing it misses (there is a test for exactly this).
- Time windows are evaluated and penalized, not enforced during construction.
- Pickup-and-delivery pairing, driver break rules, and multi-day horizons are
  not modelled yet.

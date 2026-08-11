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

**Route sequencing stays classical** (nearest neighbour, 2-opt, or-opt). A
permutation-encoded TSP QUBO needs `n²` variables and heavy penalty tuning to
compete with heuristics that solve small tours essentially exactly. Sending it
to an annealer would be theatre.

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

Only a **feasible** plan can win a benchmark. A cheaper plan that strands a
delivery or overloads a truck is not a cheaper plan.

Example output on the bundled São Paulo instance:

```
baseline     dist=78.0km cost=267.28 vehicles=3 co2=48.9kg feasible=false
classical    dist=67.9km cost=241.38 vehicles=3 co2=38.6kg feasible=true   saves 10.0 km
dwave-sa     dist=67.9km cost=241.38 vehicles=3 co2=38.6kg feasible=true   saves 10.0 km
winner: classical
```

The customer's own plan is infeasible here (it overloads a truck), both
optimized lanes agree, and neither quantum lane is better. That is a real
result, and the architecture reports it rather than hiding it.

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

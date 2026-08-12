# QuantumClaw roadmap

What is known to be missing, why it was left, and what it would cost. Written
so the work can be picked up cold.

Two rules this document tries to keep:

- **Nothing here is a promise.** It is a record of judgement calls, each with
  the reasoning attached, so a future decision can disagree with a past one on
  the merits.
- **Claims are checked against the code**, not remembered. Where an item says
  something is missing, it was verified before being written down.

Ordering principle: *validate what already ships before adding more surface,
and fix what can mislead before adding what is merely absent.*

---

## 1. Validate what is already published

### 1.1 Live D-Wave Leap and QPU execution — **blocked on credentials**

`dwave-hybrid` and `dwave-qpu` have shipped in four public releases and have
**never run against live D-Wave hardware or the Leap service.**

- Implemented and tested: configuration, request construction, and every
  failure path — missing credentials, authentication failure, solver
  unavailable, embedding failure, timeout, problem too large.
- Unverified: every success path, plus the fields only real hardware
  populates — `qpu_access_time_us`, `chain_break_fraction`,
  `hybrid_run_time_us`, `charge_time_us`. These are parsed but have never
  been observed with a value in them.

Blocked solely on a Leap API token; the free tier is sufficient for the
instance sizes here. This is the highest-value open item because it is the
only one that converts already-shipped untested code into tested code.

**Size:** an afternoon once a token exists.

---

## 2. Things that can mislead or produce a bad plan

These rank above new features. A missing capability is visible; a wrong answer
is not.

### 2.1 Route sequencing ignores time windows

`vrp::sequence` minimises distance and nothing else — it contains no reference
to time at all. Windows are evaluated *after* a route is built, and violations
are reported, but the sequencer never tries to satisfy them. On a
window-heavy instance it will happily return a distance-optimal order that is
late, when a slightly longer order would have been on time.

The KPI layer reports the lateness honestly, so this does not hide anything —
but the optimizer is not optimizing what the customer is paying for.

**Touches:** `vrp.rs` (a time-feasibility term in the improvement loop),
`constraints.rs` (shared arrival-time computation).
**Size:** a day. **Priority: highest of this section.**

### 2.2 The classical fallback cannot solve a tight bin-packing

`greedy_assignment` is cheapest-insertion. On a fleet near 100% capacity
utilisation it strands deliveries — on the bundled São Paulo instance it is
infeasible on every run, while `dwave-sa` is feasible four times in five.

That is honestly reported rather than hidden, and it is a fair demonstration
of where the optimization layer earns its place. But a first-fit-decreasing
pass with local search would close most of the gap classically, which matters
because the classical path is the fallback when no backend is available.

**Touches:** `decoder.rs`. **Size:** half a day.

### 2.3 The empirical solver-selection loop is open

`SolverRoutingPolicy` consumes a `BenchmarkLedger`, and
`QRouterBrain::ledger_records` produces records from completed runs — but
**nothing calls it.** Records are never persisted, so the policy never has
evidence and always falls through to its configured preference.

The architecture for "choose the backend that has actually performed best" is
present and tested in isolation; the loop is simply not closed. Until it is,
the empirical-selection story is a capability, not a behaviour.

**Touches:** brain solve path (emit records), a caller-supplied sink,
`RouterBenchmark` (feed results back). **Size:** half a day.

---

## 3. Domain scope named in the original brief

Both of these were explicitly requested and are genuinely incomplete.

### 3.1 Pickup-and-delivery pairing

Two requirements. Same-vehicle pairing is straightforward — the assignment
QUBO can express it directly. **Precedence is not**: the core 2-opt move is
`candidate[first..=second].reverse()`, and reversing a segment flips relative
order, so the improvement loop breaks precedence by construction.

Supporting PDP therefore means precedence-aware moves or a validity filter on
every move — a change to the sequencer, not an addition to the model. The TSP
QUBO would also need position-ordering constraints.

**Size:** ~1 day. **Unlocks:** a whole VRP class.

### 3.2 Driver break rules

The clock in `constraints.rs` is a plain accumulator of travel plus service
time. A rule such as "45 minutes rest after 4.5 hours driving" inserts rest
into that accumulation, shifting every downstream arrival, which changes
lateness, which changes SLA cost.

Not difficult, but it invalidates the expectations in every existing
time-window test.

**Size:** half a day plus test rework.

### 3.3 Multi-day horizons

Rolling-horizon *decomposition* is implemented. Genuine multi-day planning is
not: time windows are `start_min`/`end_min`, minutes from a single horizon
start. Multi-day needs absolute timestamps, overnight rest, and per-day
vehicle availability — a model change rippling through matrices, KPIs and
constraints.

**Size:** the largest item here, and arguably a different product tier.

---

## 4. Architecture and ergonomics

### 4.1 Mark output types `#[non_exhaustive]`

There are currently **zero** uses of `#[non_exhaustive]` in the workspace, so
adding a field to any public struct is a breaking change. That is what forced
0.3.0 to be a minor rather than a patch.

Marking output types — `QRouterResult`, `BenchmarkEntry`, `SolverOutput`,
`BackendTelemetry`, `RouterKpis` — would make future field additions
non-breaking. Nobody should be constructing those by hand anyway.

`#[non_exhaustive]` is itself breaking, so it belongs in the next minor bump.
**Size:** an hour. **Cheapest item here, and it pays forever.**

### 4.2 Constrained quadratic models instead of penalty encoding

Every constraint is currently compiled to a penalty term with an auto-scaled
weight. Ocean's `ConstrainedQuadraticModel` and `LeapHybridCQMSampler` accept
constraints *declaratively*, with no penalty tuning — which typically gives
better feasibility rates, exactly the weakness visible in `dwave-sa` being
feasible only four times in five.

This is the most promising unexplored direction for solution quality, and it
would also make larger sequencing models viable by removing penalty scaling
as the limiting factor.

**Touches:** a second compile target in `quantumclaw-optimization`, a new
bridge lane, a new backend. **Size:** 2–3 days. **Needs Leap credentials to
evaluate the hybrid CQM sampler**, though a local CQM path can be built first.

### 4.3 Observability integration

The `Observer` trait exists, but the D-Wave provider and Q-Router emit
**zero** events — no trace of which backend ran, what it cost, or why a
routing decision was made, beyond what is returned in the result. Structured
provider metadata is attached to telemetry, so the data exists; nothing
publishes it.

**Size:** half a day.

### 4.4 Retire the superseded quantum-inspired scaffolding

`QuboLikeProblem` and `IsingLikeMapping` in `quantumclaw-solvers-qinspired`
are referenced **nowhere outside their own crate**. They predate the real
optimization layer and are now a strictly worse duplicate of it. Same
question applies to the `quantumclaw-solvers-future-qpu` placeholder, which
the D-Wave provider supersedes.

Deleting public types is breaking, so this belongs with a minor bump, ideally
alongside 4.1.

**Size:** an hour, mostly deciding rather than typing.

### 4.5 A long-lived bridge process

One Python process is spawned per solve, costing roughly 200 ms of
interpreter startup. Mitigated already: in-solver time is measured inside
Python and reported separately from wall time, so benchmarks are not
distorted.

A persistent sidecar needs lifecycle management, health checks, request
multiplexing, crash recovery and concurrency control — real complexity for
throughput that does not currently exist.

**Deliberately deferred indefinitely.** Revisit only if someone is running
thousands of solves.

### 4.6 More domain brains

The `QuantumBrain` abstraction was built to hold more than one brain, and
Q-Router is its only implementation. Q-Scheduler, Q-Portfolio and
Q-ResourceAllocator were named as eventual candidates. The abstraction should
be considered unproven until a second brain exists — one implementation
cannot demonstrate that an interface generalises.

**Size:** per brain, comparable to Q-Router.

---

## 5. Deliberately not planned

Recorded so nobody "fixes" them by accident.

### 5.1 Non-integral inequality coefficients

Slack is encoded as a sum of binary weights, which requires integral
coefficients; fractional ones are rejected with "scale the units first".

The obvious fix — scale to a fixed resolution and round — puts rounding error
into *constraint satisfaction*. A plan could read feasible in the model while
the vehicle is 0.4 kg over capacity. For a capacity constraint that is the
worst available failure mode. Erroring pushes the tolerance decision to
whoever knows the domain.

**Keep the error.** If this is revisited, the answer is an exact rational or
scaled-integer representation, not floating-point rounding.

### 5.2 Claiming quantum advantage

No benchmark in this repository demonstrates one, and the code should keep
saying so. `dwave-sa` is classical simulated annealing over an
Ocean-compatible BQM, and `SolverKind::Classical` reflects that.

If a future benchmark does show an advantage on real hardware, it should be
published with the instance, the seed, the repetition count and the
feasibility rate — the same standard applied to every other number here.

---

## Suggested order

1. **Live D-Wave validation** (1.1) — blocked on a token, highest value.
2. **Time-window-aware sequencing** (2.1) — the optimizer should optimize what
   is being paid for.
3. **`#[non_exhaustive]` + retire dead scaffolding** (4.1, 4.4) — cheap, and
   they must ride a minor bump together.
4. **Close the ledger loop** (2.3) — makes empirical selection real.
5. **Pickup-and-delivery** (3.1) — the largest scope item actually requested.
6. **CQM support** (4.2) — the most promising quality direction.

Everything else on merit, when it becomes someone's problem.

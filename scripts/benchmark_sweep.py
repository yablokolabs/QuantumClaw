#!/usr/bin/env python3
"""Sweeps every solver backend across instance sizes and seed bases.

This is the "360-degree" benchmark: every registry backend x several problem
sizes x several seed bases, aggregated into one table. Instances are generated
deterministically (seeded), so the whole report is reproducible.

Methodology
-----------

- Delivery problems are generated with total demand tuned just under fleet
  capacity (chunky 4-6 unit demands), so bin-packing is genuinely stressed and
  feasibility — the benchmark's first ranking key — discriminates between
  lanes instead of everyone being feasible every run.
- Each candidate runs ``--repeat`` times per seed base through the ``qrouter
  benchmark`` CLI, which ranks on feasibility rate first, then median
  objective, and reports best/median/worst/mean/std across the repeats.
- Results are aggregated here per (instance size, backend) across seed bases.
- A showcase block then highlights where the quantum-inspired lane (`dwave-sqa`)
  won, using the same feasibility-then-median rule as the CLI. Winners are
  never edited or re-ranked; when the lane won nothing, the report says so.

Caveat surfaced automatically
-----------------------------

The Q-Router brain compiles every subproblem to a QUBO and asks the solver
registry for the requested backend. The classical solver family
(``greedy-classical``, ``beam-search-classical``, ``branch-and-bound-classical``,
...) does not support quadratic models, so those backends return no
optimization result and the brain falls back to its internal
cheapest-insertion path. The benchmark report does not show that fallback, so
every classical-named row looks identical. This script detects the tie and
says so rather than presenting it as a comparison of distinct solvers. Only
``classical`` (the brain's own path), ``dwave-sa``, and ``dwave-sqa`` are
distinct lanes today.

Usage
-----

    # Everything: all backends, sizes 5-10, two seed bases
    python3 scripts/benchmark_sweep.py

    # A narrower run, kept fast enough for a smoke test
    python3 scripts/benchmark_sweep.py --sizes 5,7 --seeds 1 --repeat 3 \
        --backends classical,dwave-sa,dwave-sqa

Requires Ocean on ``$QUANTUMCLAW_DWAVE_PYTHON`` for the D-Wave lanes; without
it those lanes appear as errors in the table and the rest still runs.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

VEHICLES = [
    {
        "id": "truck-1", "depot_id": "depot", "capacity": 12, "cost_per_km": 1.1,
        "fixed_cost": 40.0, "fuel_l_per_100km": 26.0, "co2_g_per_km": 690.0,
        "average_speed_kmh": 35.0,
    },
    {
        "id": "truck-2", "depot_id": "depot", "capacity": 12, "cost_per_km": 1.1,
        "fixed_cost": 40.0, "fuel_l_per_100km": 26.0, "co2_g_per_km": 690.0,
        "average_speed_kmh": 35.0,
    },
    {
        "id": "van-1", "depot_id": "depot", "capacity": 8, "cost_per_km": 0.7,
        "fixed_cost": 15.0, "fuel_l_per_100km": 12.0, "co2_g_per_km": 310.0,
        "average_speed_kmh": 40.0,
    },
]

DEFAULT_BACKENDS = [
    "classical",
    "greedy-classical",
    "beam-search-classical",
    "heuristic-search-classical",
    "branch-and-bound-classical",
    "simulated-annealing-classical",
    "evolutionary-classical",
    "quantum-inspired-hybrid",
    "dwave-sa",
    "dwave-sqa",
]

#: Classical-family names that all resolve to the brain's greedy fallback today.
CLASSICAL_FAMILY = {
    "greedy-classical",
    "beam-search-classical",
    "heuristic-search-classical",
    "branch-and-bound-classical",
    "simulated-annealing-classical",
    "evolutionary-classical",
    "quantum-inspired-hybrid",
}


def make_problem(n: int, workdir: Path, seed: int = 1_000, capacity_slack: int = 2) -> Path:
    """Generates a deterministic delivery problem with `n` stops.

    Total demand is tuned to ``fleet_capacity - capacity_slack`` so the
    instance is a tight bin-packing problem: feasible only when every vehicle
    is nearly full, which is what makes the feasibility ranking bite.
    """
    rng = random.Random(seed + n)
    depot = {"id": "depot", "name": "depot", "location": {"lat": -23.55, "lon": -46.63}}
    deliveries = []
    for index in range(n):
        deliveries.append(
            {
                "id": f"stop-{index}",
                "location": {
                    "lat": round(-23.55 + rng.uniform(-0.20, 0.20), 6),
                    "lon": round(-46.63 + rng.uniform(-0.20, 0.20), 6),
                },
                # Chunky items (4-6) make exact packing genuinely hard.
                "demand": 4 + rng.randint(0, 2),
                "service_time_min": 10,
            }
        )

    capacity = sum(vehicle["capacity"] for vehicle in VEHICLES)
    target = capacity - capacity_slack
    total = sum(delivery["demand"] for delivery in deliveries)
    # Trim the heaviest items until the total lands at the target.
    while total > target:
        heaviest = max(deliveries, key=lambda delivery: delivery["demand"])
        heaviest["demand"] -= 1
        total = sum(delivery["demand"] for delivery in deliveries)
    # If the target is still missed because items are chunky, trim the last item.
    while total < target:
        deliveries[-1]["demand"] += 1
        total = sum(delivery["demand"] for delivery in deliveries)

    problem = {
        "problem": {
            "id": f"sweep-{n}-stops",
            "depots": [depot],
            "vehicles": VEHICLES,
            "deliveries": deliveries,
            "matrix": {"kind": "haversine", "average_speed_kmh": 32.0},
            "cost_model": {"fuel_price_per_liter": 1.45, "driver_cost_per_hour": 18.0},
            "sla": {"late_penalty_per_minute": 2.0, "breach_after_minutes": 30.0},
        },
        "options": {"max_variables_per_subproblem": 40},
    }
    path = workdir / f"problem-{n}.json"
    path.write_text(json.dumps(problem, indent=2))
    return path


def run_benchmark(
    path: Path,
    seed: int,
    backends: list[str],
    repeat: int,
    python: str,
) -> dict:
    """Runs one `qrouter benchmark` invocation and returns the report JSON."""
    command = [
        "cargo", "run", "-q", "-p", "quantumclaw-app", "--bin", "quantumclaw",
        "qrouter", "benchmark", str(path),
        "--backends", ",".join(backends),
        "--repeat", str(repeat),
        "--seed", str(seed),
    ]
    env = dict(os.environ)
    env["QUANTUMCLAW_DWAVE_PYTHON"] = python
    completed = subprocess.run(
        command, cwd=REPO, capture_output=True, text=True, env=env
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"benchmark failed for {path.name} seed {seed}: {completed.stderr[-500:]}"
        )
    return json.loads(completed.stdout)


def aggregate(rows: list[tuple]) -> dict[tuple[int, str], dict]:
    """Collects (size, backend) cells across seed bases."""
    cells: dict[tuple[int, str], dict] = defaultdict(
        lambda: {"feasible": [], "median": [], "time": []}
    )
    for size, backend, error, feasible, median, runtime in rows:
        cell = cells[(size, backend)]
        if error is not None:
            cell["error"] = error
            continue
        cell["feasible"].append(feasible)
        if median is not None:
            cell["median"].append(median)
        if runtime is not None:
            cell["time"].append(runtime)
    return cells


def pick_winner(cells: dict, order: list[str], size: int, seed_index: int):
    """Winner for one (size, seed base) cell: feasibility rate, then median.

    The same rule the `qrouter benchmark` CLI itself uses, so the showcase
    cannot disagree with the underlying report.
    """
    candidates = []
    for name in order:
        cell = cells.get((size, name))
        if cell is None or "error" in cell:
            continue
        if seed_index >= len(cell["feasible"]) or seed_index >= len(cell["median"]):
            continue
        feasible = int(cell["feasible"][seed_index].split("/")[0])
        if feasible == 0:
            continue
        candidates.append((name, feasible, cell["median"][seed_index]))
    if not candidates:
        return None
    return max(candidates, key=lambda item: (item[1], -item[2]))


def print_showcase(
    cells: dict, order: list[str], sizes: list[int], seed_bases: int, lane: str = "dwave-sqa"
) -> None:
    """Highlights where the quantum-inspired lane won, on measured results.

    Nothing here edits or re-ranks the sweep: the winners come from the same
    feasibility-then-median rule the CLI uses. If the lane won nowhere, that is
    reported too, so the showcase can never drift into a fabricated win.
    """
    print()
    print(f"=== {lane} showcase (measured results, unedited) ===")
    wins = [
        (size, seed_index)
        for size in sizes
        for seed_index in range(seed_bases)
        if pick_winner(cells, order, size, seed_index) is not None
        and pick_winner(cells, order, size, seed_index)[0] == lane
    ]
    if not wins:
        showings = [
            (size, seed_index, int(cells[(size, lane)]["feasible"][seed_index].split("/")[0]),
             cells[(size, lane)]["median"][seed_index])
            for size in sizes
            if (size, lane) in cells and "median" in cells[(size, lane)]
            for seed_index in range(seed_bases)
            if seed_index < len(cells[(size, lane)]["median"])
        ]
        if not showings:
            print(f"  {lane} did not run or produced no feasible results here.")
            return
        size, seed_index, feasible, median = max(
            showings, key=lambda item: (item[2], -item[3])
        )
        winner = pick_winner(cells, order, size, seed_index)
        winner_name = winner[0] if winner else "no feasible candidate"
        print(
            f"  {lane} did not win any size in this run; closest at {size} stops "
            f"(median {median}, feasible {feasible}/"
            f"{cells[(size, lane)]['feasible'][seed_index].split('/')[1]}), "
            f"where {winner_name} won."
        )
        return
    for size, seed_index in wins:
        print(f"  {lane} won at {size} stops (seed base {seed_index + 1}):")
        for name in (lane, "dwave-sa", "classical"):
            cell = cells.get((size, name))
            if cell is None or "error" in cell or seed_index >= len(cell["median"]):
                continue
            print(
                f"    {name:<12} median {cell['median'][seed_index]:<9} feasible "
                f"{cell['feasible'][seed_index]}"
            )


def print_report(cells: dict[tuple[int, str], dict], sizes: list[int], order: list[str]) -> None:
    header = (
        f"{'stops':>5} {'backend':<28} {'feasible':>20} "
        f"{'median per seed base':>24} {'solver ms':>10}"
    )
    print(header)
    print("-" * len(header))
    for size in sizes:
        for backend in order:
            cell = cells.get((size, backend))
            if cell is None:
                continue
            if "error" in cell:
                print(f"{size:>5} {backend:<28} {'error: ' + cell['error'][:44]}")
                continue
            feasible = " ".join(cell["feasible"])
            medians = " / ".join(str(median) for median in cell["median"])
            runtime = (
                f"{sum(cell['time']) / len(cell['time']):.0f}" if cell["time"] else "-"
            )
            print(f"{size:>5} {backend:<28} {feasible:>20} {medians:>24} {runtime:>10}")
        print()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sizes", default="5,6,7,8,10", help="comma-separated stop counts")
    parser.add_argument("--seeds", default="1,101", help="comma-separated seed bases")
    parser.add_argument("--backends", default=",".join(DEFAULT_BACKENDS))
    parser.add_argument("--repeat", type=int, default=5)
    parser.add_argument("--capacity-slack", type=int, default=2,
                        help="total demand sits this far under fleet capacity")
    parser.add_argument("--workdir", default="/tmp/qc_sweep_data",
                        help="where generated problems and reports live")
    arguments = parser.parse_args()

    sizes = [int(value) for value in arguments.sizes.split(",")]
    seeds = [int(value) for value in arguments.seeds.split(",")]
    backends = [value.strip() for value in arguments.backends.split(",") if value.strip()]
    # Bridge interpreter: the caller's configured one, or python3.
    python = os.environ.get("QUANTUMCLAW_DWAVE_PYTHON", "python3")

    workdir = Path(arguments.workdir)
    workdir.mkdir(parents=True, exist_ok=True)

    rows: list[tuple] = []
    for size in sizes:
        path = make_problem(size, workdir, capacity_slack=arguments.capacity_slack)
        for seed in seeds:
            report = run_benchmark(path, seed, backends, arguments.repeat, python)
            for entry in report["entries"]:
                label = entry["label"]
                stats = entry.get("stats")
                if stats is None:
                    rows.append((size, label, entry.get("error"), None, None, None))
                    continue
                rows.append((
                    size, label, None,
                    f"{stats['feasible_runs']}/{stats['runs']}",
                    round(stats["objective_median"], 1),
                    entry.get("solver_runtime_ms"),
                ))

    cells = aggregate(rows)
    order = sorted({row[1] for row in rows},
                   key=lambda name: (name in CLASSICAL_FAMILY, name))
    print_report(cells, sizes, order)

    # Surface the classical-family caveat when it actually happened: every
    # classical-named backend resolving to the same greedy fallback plan.
    tied = False
    for size in sizes:
        medians = [
            cells[(size, name)]["median"][0]
            for name in CLASSICAL_FAMILY
            if (size, name) in cells and "median" in cells[(size, name)]
        ]
        if len(medians) > 1 and len(set(medians)) == 1:
            tied = True
            break
    if tied:
        print(
            "note: every classical-named backend returned identical objectives, "
            "because none of them support quadratic models; the brain silently "
            "fell back to its cheapest-insertion path for all of them. Only "
            "'classical', 'dwave-sa', and 'dwave-sqa' are distinct lanes today."
        )

    # Highlight where the quantum-inspired lane won, on measured results.
    print_showcase(cells, order, sizes, len(seeds))
    return 0


if __name__ == "__main__":
    sys.exit(main())

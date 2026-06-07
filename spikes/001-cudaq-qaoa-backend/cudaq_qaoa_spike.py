#!/usr/bin/env python3
# pyright: reportUndefinedVariable=false, reportInvalidTypeForm=false
"""CUDA-Q sidecar spike for QuantumClaw QUBO-like planning problems.

The script accepts the same minimal shape emitted by the current
`quantumclaw-solvers-qinspired::QuboLikeProblem` scaffold:

{
  "variables": ["inspect", "test-first", "small-edit", "validate"],
  "linear_weights": [0.78, 0.90, 0.86, 0.93],
  "pairwise_penalties": [[0, 1, -0.10], [1, 2, -0.10]],
  "qaoa_layers": 1,
  "shots": 512
}

If CUDA-Q is installed, this runs a small weighted-Ising/QAOA experiment. If it is
not installed, the script still validates the sidecar contract and returns a
classical fallback result so CI/developers can exercise the spike on normal hosts.
"""

from __future__ import annotations

import argparse
import itertools
import json
import math
import random
from pathlib import Path
from typing import Any, Iterable

try:
    import cudaq as _cudaq  # type: ignore[import-not-found]
    from cudaq import spin as _cudaq_spin  # type: ignore[import-not-found]
except ImportError as exc:
    _cudaq = None
    _cudaq_spin = None
    _cudaq_import_error: str | None = str(exc)
else:
    _cudaq_import_error = None


def load_problem(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        problem = json.load(handle)

    variables = problem.get("variables")
    weights = problem.get("linear_weights")
    penalties = problem.get("pairwise_penalties", [])

    if not isinstance(variables, list) or not variables:
        raise ValueError("problem.variables must be a non-empty list")
    if not isinstance(weights, list) or len(weights) != len(variables):
        raise ValueError("problem.linear_weights must match variables length")
    for entry in penalties:
        if not (isinstance(entry, list) and len(entry) == 3):
            raise ValueError("each pairwise penalty must be [source, target, weight]")
        src, tgt, _weight = entry
        if not (0 <= int(src) < len(variables) and 0 <= int(tgt) < len(variables)):
            raise ValueError(f"pairwise penalty references invalid variable index: {entry}")

    return problem


def score_bitstring(bits: Iterable[int], problem: dict[str, Any]) -> float:
    bit_values = list(bits)
    score = sum(
        float(weight) * bit_values[index]
        for index, weight in enumerate(problem["linear_weights"])
    )
    for src, tgt, penalty in problem.get("pairwise_penalties", []):
        score += float(penalty) * bit_values[int(src)] * bit_values[int(tgt)]
    return score


def selected_actions(bitstring: str, problem: dict[str, Any]) -> list[str]:
    return [
        str(variable)
        for bit, variable in zip(bitstring, problem["variables"])
        if bit == "1"
    ]


def brute_force_or_greedy(problem: dict[str, Any]) -> dict[str, Any]:
    n_variables = len(problem["variables"])
    if n_variables <= 20:
        candidates = (
            "".join(str(bit) for bit in bits)
            for bits in itertools.product([0, 1], repeat=n_variables)
        )
        evaluated = [
            (candidate, score_bitstring((int(bit) for bit in candidate), problem))
            for candidate in candidates
        ]
        best_bitstring, best_score = max(evaluated, key=lambda item: item[1])
        return {
            "solver": "classical-bruteforce",
            "candidate_count": len(evaluated),
            "best_bitstring": best_bitstring,
            "best_score": best_score,
        }

    # Keep the fallback bounded for large spike inputs.
    ordered = sorted(
        enumerate(problem["linear_weights"]), key=lambda item: float(item[1]), reverse=True
    )
    active = {index for index, weight in ordered if float(weight) > 0}
    bitstring = "".join("1" if index in active else "0" for index in range(n_variables))
    return {
        "solver": "classical-greedy-large-n",
        "candidate_count": None,
        "best_bitstring": bitstring,
        "best_score": score_bitstring((int(bit) for bit in bitstring), problem),
    }


def qubo_to_ising_for_minimized_negative_objective(
    problem: dict[str, Any],
) -> tuple[list[float], list[int], list[int], list[float]]:
    """Map max QUBO objective to an Ising Hamiltonian to minimize.

    QUBO objective: maximize sum_i w_i x_i + sum_ij J_ij x_i x_j.
    With x_i = (1 - z_i) / 2, minimizing -objective yields the non-constant
    coefficients below. Constants are dropped because they do not affect argmin.
    """

    fields = [0.5 * float(weight) for weight in problem["linear_weights"]]
    edges_src: list[int] = []
    edges_tgt: list[int] = []
    couplings: list[float] = []

    for src_raw, tgt_raw, penalty_raw in problem.get("pairwise_penalties", []):
        src = int(src_raw)
        tgt = int(tgt_raw)
        penalty = float(penalty_raw)
        fields[src] += 0.25 * penalty
        fields[tgt] += 0.25 * penalty
        edges_src.append(src)
        edges_tgt.append(tgt)
        couplings.append(-0.25 * penalty)

    return fields, edges_src, edges_tgt, couplings


def cudaq_import_report() -> dict[str, Any]:
    """Return a machine-readable CUDA-Q import/status probe."""

    if _cudaq is None:
        return {
            "cudaq_available": False,
            "reason": f"CUDA-Q unavailable: {_cudaq_import_error}",
        }

    report: dict[str, Any] = {
        "cudaq_available": True,
        "cudaq_version": getattr(_cudaq, "__version__", "unknown"),
    }
    try:
        report["available_gpus"] = _cudaq.num_available_gpus()
    except Exception as exc:
        report["available_gpus_error"] = f"{type(exc).__name__}: {exc}"
    try:
        report["has_nvidia_target"] = _cudaq.has_target("nvidia")
    except Exception as exc:
        report["has_nvidia_target_error"] = f"{type(exc).__name__}: {exc}"
    return report


def run_cudaq_qaoa(problem: dict[str, Any]) -> dict[str, Any]:
    if _cudaq is None or _cudaq_spin is None:
        raise ModuleNotFoundError(_cudaq_import_error or "No module named 'cudaq'")

    cudaq = _cudaq
    spin = _cudaq_spin
    from typing import List

    try:
        if cudaq.num_available_gpus() > 0 and cudaq.has_target("nvidia"):
            cudaq.set_target("nvidia")
            target = "nvidia"
        else:
            cudaq.set_target("qpp-cpu")
            target = "qpp-cpu"
    except Exception:
        # Older/newer CUDA-Q builds may differ in target helpers; leave default intact.
        target = "default"

    fields, edges_src, edges_tgt, couplings = qubo_to_ising_for_minimized_negative_objective(
        problem
    )
    qubit_count = len(problem["variables"])
    layer_count = int(problem.get("qaoa_layers", 1))
    shots = int(problem.get("shots", 512))
    parameter_count = 2 * layer_count

    hamiltonian = 0
    for index, coefficient in enumerate(fields):
        hamiltonian += coefficient * spin.z(index)
    for src, tgt, coefficient in zip(edges_src, edges_tgt, couplings):
        hamiltonian += coefficient * spin.z(src) * spin.z(tgt)

    @cudaq.kernel
    def weighted_pair(q0: cudaq.qubit, q1: cudaq.qubit, gamma: float, coefficient: float):
        x.ctrl(q0, q1)
        rz(2.0 * gamma * coefficient, q1)
        x.ctrl(q0, q1)

    @cudaq.kernel
    def qaoa_kernel(
        n_qubits: int,
        n_layers: int,
        field_coefficients: List[float],
        src_indices: List[int],
        tgt_indices: List[int],
        pair_coefficients: List[float],
        thetas: List[float],
    ):
        qreg = cudaq.qvector(n_qubits)
        h(qreg)

        for layer in range(n_layers):
            gamma = thetas[layer]
            beta = thetas[layer + n_layers]
            for qubit in range(n_qubits):
                rz(2.0 * gamma * field_coefficients[qubit], qreg[qubit])
            for edge_index in range(len(src_indices)):
                weighted_pair(
                    qreg[src_indices[edge_index]],
                    qreg[tgt_indices[edge_index]],
                    gamma,
                    pair_coefficients[edge_index],
                )
            for qubit in range(n_qubits):
                rx(2.0 * beta, qreg[qubit])

    random.seed(13)
    optimizer = cudaq.optimizers.NelderMead()
    optimizer.initial_parameters = [
        random.uniform(-math.pi / 8, math.pi / 8) for _ in range(parameter_count)
    ]

    def objective(parameters: list[float]) -> float:
        return cudaq.observe(
            qaoa_kernel,
            hamiltonian,
            qubit_count,
            layer_count,
            fields,
            edges_src,
            edges_tgt,
            couplings,
            parameters,
        ).expectation()

    optimal_expectation, optimal_parameters = optimizer.optimize(
        dimensions=parameter_count, function=objective
    )
    counts = cudaq.sample(
        qaoa_kernel,
        qubit_count,
        layer_count,
        fields,
        edges_src,
        edges_tgt,
        couplings,
        optimal_parameters,
        shots_count=shots,
    )

    # CUDA-Q sample results act like a dict of bitstring -> count.
    bitstrings = list(counts)
    if not bitstrings:
        raise RuntimeError("CUDA-Q returned no samples")
    best_bitstring = max(
        bitstrings,
        key=lambda bitstring: score_bitstring((int(bit) for bit in str(bitstring)), problem),
    )
    best_bitstring = str(best_bitstring)

    return {
        "solver": "cudaq-qaoa",
        "target": target,
        "layers": layer_count,
        "shots": shots,
        "optimal_expectation": float(optimal_expectation),
        "optimal_parameters": [float(value) for value in optimal_parameters],
        "sample_count": len(bitstrings),
        "best_bitstring": best_bitstring,
        "best_score": score_bitstring((int(bit) for bit in best_bitstring), problem),
    }


def solve(problem: dict[str, Any], *, require_cudaq: bool = False) -> dict[str, Any]:
    try:
        result = run_cudaq_qaoa(problem)
        mode = "cudaq"
        cudaq_available = True
        reason = None
    except ModuleNotFoundError as exc:
        if require_cudaq:
            raise
        result = brute_force_or_greedy(problem)
        mode = "classical-fallback"
        cudaq_available = False
        reason = f"CUDA-Q unavailable: {exc}"
    except Exception as exc:
        if require_cudaq:
            raise
        result = brute_force_or_greedy(problem)
        mode = "classical-fallback"
        cudaq_available = False
        reason = f"CUDA-Q path failed; fallback used: {type(exc).__name__}: {exc}"

    bitstring = result["best_bitstring"]
    return {
        "mode": mode,
        "cudaq_available": cudaq_available,
        "reason": reason,
        "variables": problem["variables"],
        **result,
        "selected_actions": selected_actions(bitstring, problem),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "problem",
        nargs="?",
        type=Path,
        default=Path(__file__).with_name("sample_problem.json"),
        help="Path to a QUBO-like QuantumClaw problem JSON file.",
    )
    parser.add_argument(
        "--check-cudaq-import",
        action="store_true",
        help="Probe `import cudaq` and print CUDA-Q availability without solving.",
    )
    parser.add_argument(
        "--require-cudaq",
        action="store_true",
        help="Fail instead of using the classical fallback when CUDA-Q is unavailable.",
    )
    args = parser.parse_args()

    if args.check_cudaq_import:
        report = cudaq_import_report()
        print(json.dumps(report, indent=2, sort_keys=True))
        if args.require_cudaq and not report["cudaq_available"]:
            raise SystemExit(1)
        return

    problem = load_problem(args.problem)
    try:
        print(json.dumps(solve(problem, require_cudaq=args.require_cudaq), indent=2, sort_keys=True))
    except Exception as exc:
        print(
            json.dumps(
                {
                    "cudaq_available": False,
                    "mode": "cudaq-required",
                    "reason": f"CUDA-Q execution failed: {type(exc).__name__}: {exc}",
                },
                indent=2,
                sort_keys=True,
            )
        )
        raise SystemExit(1) from exc


if __name__ == "__main__":
    main()

"""Ocean sampler execution for the QuantumClaw D-Wave bridge.

Five execution lanes are exposed:

``simulated_annealing``
    ``dwave.samplers.SimulatedAnnealingSampler``. This is **classical**
    simulated annealing running on the local CPU over an Ocean-compatible BQM.
    It is not a simulator of a D-Wave QPU and makes no quantum claim.
``exact``
    ``dimod.ExactSolver``. Classical exhaustive search, for validating small
    models and checking heuristic results.
``hybrid``
    ``dwave.system.LeapHybridSampler``. D-Wave's managed hybrid
    quantum/classical solver. Requires Leap credentials.
``qpu``
    ``dwave.system.DWaveSampler`` behind ``EmbeddingComposite``. Quantum
    annealing hardware. Requires Leap credentials and a working minor embedding.
``simulated_quantum_annealing``
    ``dwave.samplers.PathIntegralAnnealingSampler``. A **local emulator** of
    quantum annealing dynamics running on the CPU. It is not a QPU and its
    results must never be described as real quantum results.
"""

from __future__ import annotations

import os
import time
from typing import Any

from .errors import (
    AUTHENTICATION_FAILED,
    INSTALL_HINT,
    EMBEDDING_FAILED,
    INVALID_CONFIGURATION,
    MISSING_CREDENTIALS,
    NO_FEASIBLE_RESULT,
    OCEAN_MISSING,
    PROBLEM_TOO_LARGE,
    SAMPLER_FAILED,
    SOLVER_UNAVAILABLE,
    TIMEOUT,
    BridgeError,
)
from .models import BridgeRequest, build_dimod_bqm

DEFAULT_EXACT_MAX_VARIABLES = 20


def _import(module: str, extra: str = "dwave"):
    try:
        return __import__(module, fromlist=["*"])
    except ImportError as exc:
        raise BridgeError(
            OCEAN_MISSING,
            f"{INSTALL_HINT} (missing module '{module}', provided by the '{extra}' extra)",
            exc,
        ) from exc


def _jsonable(value: Any) -> Any:
    """Converts numpy scalars and containers into JSON-safe values."""

    if isinstance(value, dict):
        return {str(key): _jsonable(item) for key, item in value.items()}
    if isinstance(value, (list, tuple, set)):
        return [_jsonable(item) for item in value]
    if isinstance(value, (str, bool)) or value is None:
        return value
    if isinstance(value, int):
        return int(value)
    if isinstance(value, float):
        return float(value)
    item = getattr(value, "item", None)
    if callable(item):
        try:
            return _jsonable(item())
        except (TypeError, ValueError):
            pass
    tolist = getattr(value, "tolist", None)
    if callable(tolist):
        try:
            return _jsonable(tolist())
        except (TypeError, ValueError):
            pass
    return str(value)


def _credential_config(options: dict[str, Any]) -> dict[str, Any]:
    """Non-secret connection settings.

    The API token is deliberately never read or forwarded here. Ocean picks it
    up from ``DWAVE_API_TOKEN`` or the user's ``dwave.conf`` on its own, so the
    secret never passes through QuantumClaw.
    """

    config = {
        key: options.get(key)
        for key in ("solver", "region", "endpoint", "profile")
        if options.get(key)
    }
    return config


def _require_credentials(options: dict[str, Any]) -> None:
    if os.environ.get("DWAVE_API_TOKEN"):
        return
    try:
        from dwave.cloud import config as cloud_config

        loaded = cloud_config.load_config(profile=options.get("profile"))
    except Exception:  # noqa: BLE001 - absence of config is the normal case here
        loaded = {}
    if loaded.get("token"):
        return
    raise BridgeError(
        MISSING_CREDENTIALS,
        "no D-Wave Leap credentials found. Set DWAVE_API_TOKEN or run 'dwave config create'.",
    )


def _translate(exc: BaseException) -> BridgeError:
    """Maps an Ocean exception onto a stable bridge error code."""

    if isinstance(exc, BridgeError):
        return exc

    name = type(exc).__name__
    text = str(exc).lower()

    if name in {"SolverAuthenticationError", "UnauthorizedError"}:
        return BridgeError(AUTHENTICATION_FAILED, "D-Wave Leap rejected the credentials", exc)
    if name in {"SolverNotFoundError", "SolverOfflineError", "SolverError", "InvalidSolverError"}:
        return BridgeError(SOLVER_UNAVAILABLE, "the requested D-Wave solver is unavailable", exc)
    if name in {"RequestTimeout", "PollingTimeout"} or "timed out" in text:
        return BridgeError(TIMEOUT, "the D-Wave request timed out", exc)
    if "embedding" in text or name in {"EmbeddingError", "NoEmbeddingFoundError"}:
        return BridgeError(
            EMBEDDING_FAILED,
            "the problem could not be embedded onto the QPU topology",
            exc,
        )
    if isinstance(exc, (ConnectionError, OSError)):
        return BridgeError(SOLVER_UNAVAILABLE, "could not reach the D-Wave service", exc)
    return BridgeError(SAMPLER_FAILED, f"the D-Wave sampler failed: {exc}", exc)


def _summarize(sampleset, backend: str, sampler_name: str, request: BridgeRequest, elapsed_ms: float) -> dict[str, Any]:
    if len(sampleset) == 0:
        raise BridgeError(NO_FEASIBLE_RESULT, "the sampler returned no samples")

    first = sampleset.first
    sample = {str(key): int(value) for key, value in first.sample.items()}
    info = _jsonable(dict(getattr(sampleset, "info", {}) or {}))
    timing = info.get("timing") if isinstance(info.get("timing"), dict) else {}

    result: dict[str, Any] = {
        "backend": backend,
        "sampler": sampler_name,
        "problem_type": "BQM",
        "problem_id": request.problem_id,
        "num_variables": request.bqm.num_variables,
        "num_interactions": request.bqm.num_interactions,
        "num_samples": int(len(sampleset)),
        "solver_runtime_ms": round(elapsed_ms, 3),
        "best": {
            "sample": sample,
            "energy": float(first.energy),
            "num_occurrences": int(getattr(first, "num_occurrences", 1)),
        },
        "info": info,
    }

    num_reads = request.parameters.get("num_reads")
    if num_reads is not None:
        result["num_reads"] = int(num_reads)

    qpu_access_time = timing.get("qpu_access_time") or info.get("qpu_access_time")
    if qpu_access_time is not None:
        result["qpu_access_time_us"] = float(qpu_access_time)
    run_time = info.get("run_time")
    if run_time is not None:
        result["run_time_us"] = float(run_time)
    charge_time = info.get("charge_time")
    if charge_time is not None:
        result["charge_time_us"] = float(charge_time)

    chain_break = getattr(first, "chain_break_fraction", None)
    if chain_break is not None:
        result["chain_break_fraction"] = float(chain_break)
    elif "chain_break_fraction" in getattr(sampleset.record, "dtype", ()).names or ():
        fractions = sampleset.record.chain_break_fraction
        result["chain_break_fraction"] = float(sum(fractions) / len(fractions))

    return result


def _sample_simulated_annealing(request: BridgeRequest, bqm) -> tuple[Any, str, float]:
    module = _import("dwave.samplers", extra="local")
    sampler = module.SimulatedAnnealingSampler()

    kwargs: dict[str, Any] = {"num_reads": int(request.parameters.get("num_reads", 100))}
    if request.parameters.get("num_sweeps") is not None:
        kwargs["num_sweeps"] = int(request.parameters["num_sweeps"])
    if request.parameters.get("seed") is not None:
        kwargs["seed"] = int(request.parameters["seed"])
    beta_range = request.parameters.get("beta_range")
    if beta_range is not None:
        if not (isinstance(beta_range, (list, tuple)) and len(beta_range) == 2):
            raise BridgeError(
                INVALID_CONFIGURATION, f"beta_range must be a two-element list, got {beta_range!r}"
            )
        kwargs["beta_range"] = [float(beta_range[0]), float(beta_range[1])]

    started = time.perf_counter()
    sampleset = sampler.sample(bqm, **kwargs)
    sampleset.resolve()
    return sampleset, "dwave.samplers.SimulatedAnnealingSampler", (time.perf_counter() - started) * 1000.0


def _sample_exact(request: BridgeRequest, bqm) -> tuple[Any, str, float]:
    limit = int(request.options.get("max_variables") or DEFAULT_EXACT_MAX_VARIABLES)
    if request.bqm.num_variables > limit:
        raise BridgeError(
            PROBLEM_TOO_LARGE,
            f"the exact solver enumerates 2^n assignments; {request.bqm.num_variables} variables "
            f"exceeds the configured limit of {limit}",
        )

    module = _import("dimod", extra="local")
    started = time.perf_counter()
    sampleset = module.ExactSolver().sample(bqm)
    return sampleset, "dimod.ExactSolver", (time.perf_counter() - started) * 1000.0


def _sample_hybrid(request: BridgeRequest, bqm) -> tuple[Any, str, float]:
    _require_credentials(request.options)
    module = _import("dwave.system")
    sampler = module.LeapHybridSampler(**_credential_config(request.options))

    kwargs: dict[str, Any] = {}
    if request.parameters.get("time_limit_s") is not None:
        kwargs["time_limit"] = float(request.parameters["time_limit_s"])
    if request.parameters.get("label"):
        kwargs["label"] = str(request.parameters["label"])

    started = time.perf_counter()
    sampleset = sampler.sample(bqm, **kwargs)
    sampleset.resolve()
    return sampleset, "dwave.system.LeapHybridSampler", (time.perf_counter() - started) * 1000.0


def _sample_qpu(request: BridgeRequest, bqm) -> tuple[Any, str, float]:
    _require_credentials(request.options)
    module = _import("dwave.system")
    qpu = module.DWaveSampler(**_credential_config(request.options))
    sampler = module.EmbeddingComposite(qpu)

    kwargs: dict[str, Any] = {
        "num_reads": int(request.parameters.get("num_reads", 100)),
        "return_embedding": True,
    }
    if request.parameters.get("chain_strength") is not None:
        kwargs["chain_strength"] = float(request.parameters["chain_strength"])
    if request.parameters.get("annealing_time_us") is not None:
        kwargs["annealing_time"] = float(request.parameters["annealing_time_us"])
    if request.parameters.get("label"):
        kwargs["label"] = str(request.parameters["label"])

    started = time.perf_counter()
    sampleset = sampler.sample(bqm, **kwargs)
    sampleset.resolve()
    return sampleset, "dwave.system.EmbeddingComposite(DWaveSampler)", (time.perf_counter() - started) * 1000.0


def _sample_simulated_quantum_annealing(request: BridgeRequest, bqm) -> tuple[Any, str, float]:
    module = _import("dwave.samplers", extra="local")
    sampler = module.PathIntegralAnnealingSampler()

    kwargs: dict[str, Any] = {"num_reads": int(request.parameters.get("num_reads", 100))}
    if request.parameters.get("num_sweeps") is not None:
        kwargs["num_sweeps"] = int(request.parameters["num_sweeps"])
    if request.parameters.get("seed") is not None:
        kwargs["seed"] = int(request.parameters["seed"])
    beta_range = request.parameters.get("beta_range")
    if beta_range is not None:
        if not (isinstance(beta_range, (list, tuple)) and len(beta_range) == 2):
            raise BridgeError(
                INVALID_CONFIGURATION, f"beta_range must be a two-element list, got {beta_range!r}"
            )
        kwargs["beta_range"] = [float(beta_range[0]), float(beta_range[1])]

    started = time.perf_counter()
    sampleset = sampler.sample(bqm, **kwargs)
    sampleset.resolve()
    return sampleset, "dwave.samplers.PathIntegralAnnealingSampler", (time.perf_counter() - started) * 1000.0


_LANES = {
    "simulated_annealing": _sample_simulated_annealing,
    "exact": _sample_exact,
    "hybrid": _sample_hybrid,
    "qpu": _sample_qpu,
    "simulated_quantum_annealing": _sample_simulated_quantum_annealing,
}


def run(request: BridgeRequest) -> dict[str, Any]:
    """Executes one sampling request and returns the result payload."""

    bqm = build_dimod_bqm(request.bqm)
    lane = _LANES[request.backend]
    try:
        sampleset, sampler_name, elapsed_ms = lane(request, bqm)
        return _summarize(sampleset, request.backend, sampler_name, request, elapsed_ms)
    except BridgeError:
        raise
    except Exception as exc:  # noqa: BLE001 - every provider failure is translated
        raise _translate(exc) from exc


def probe() -> dict[str, Any]:
    """Reports which Ocean components are importable on this interpreter."""

    available: dict[str, bool] = {}
    versions: dict[str, str] = {}
    modules: dict[str, Any] = {}
    for module_name in ("dimod", "dwave.samplers", "dwave.system", "minorminer"):
        try:
            modules[module_name] = __import__(module_name, fromlist=["*"])
        except ImportError:
            available[module_name] = False
            continue
        available[module_name] = True
        version = getattr(modules[module_name], "__version__", None)
        if version:
            versions[module_name] = str(version)

    samplers = modules.get("dwave.samplers")
    return {
        "available": available,
        "versions": versions,
        "backends": {
            "simulated_annealing": available.get("dwave.samplers", False),
            "simulated_quantum_annealing": samplers is not None
            and hasattr(samplers, "PathIntegralAnnealingSampler"),
            "exact": available.get("dimod", False),
            "hybrid": available.get("dwave.system", False),
            "qpu": available.get("dwave.system", False) and available.get("minorminer", False),
        },
        "credentials_present": bool(os.environ.get("DWAVE_API_TOKEN")),
    }
